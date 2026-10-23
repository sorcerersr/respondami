//! VCC normalization — turn session entries into uniform blocks.
//!
//! Each `SessionEntry::Message` becomes zero or more [`VccBlock`]s: `user`,
//! `assistant` (text), `tool_call` (name + args), `tool_result`, and `bash`
//! (extracted from a `bash` tool call paired with its result, with exit code).
//! Thinking blocks are folded into the assistant text. Noise (empty blocks) is
//! dropped. There are no `#N` refs (respondami has no recall tool).
//!
//! Also hosts the shared text helpers (`sanitize`, `clip`, `non_empty_lines`)
//! used across the VCC modules.

use serde_json::Value;
use std::collections::HashMap;

use crate::session::{AgentMessage, ContentBlock, SessionEntry};

/// A normalized unit of conversation, independent of the wire message shape.
#[derive(Debug, Clone, PartialEq)]
pub enum VccBlock {
    /// A user turn (sanitized text).
    User { text: String },
    /// An assistant text turn (text + folded thinking).
    Assistant { text: String },
    /// A tool call (name + raw arguments).
    ToolCall { name: String, args: Value },
    /// A tool result (tool name + result text).
    ToolResult { name: String, text: String },
    /// A bash command with its output and exit code.
    Bash {
        command: String,
        output: String,
        exit_code: Option<i32>,
    },
}

/// Sanitize text: normalize line endings and strip ANSI escapes + control chars.
#[must_use]
pub fn sanitize(text: &str) -> String {
    let normalized = text.replace("\r\n", "\n").replace('\r', "\n");
    // Strip ANSI escape sequences (ESC [ ... letter).
    let mut out = String::with_capacity(normalized.len());
    let mut chars = normalized.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\x1b' {
            if chars.peek() == Some(&'[') {
                chars.next();
                while let Some(&n) = chars.peek() {
                    chars.next();
                    if n.is_ascii_alphabetic() {
                        break;
                    }
                }
                continue;
            }
            continue;
        }
        // Drop C0 control chars except tab and newline.
        if (c as u32) < 0x20 && c != '\t' && c != '\n' {
            continue;
        }
        out.push(c);
    }
    out
}

/// Clip text to at most `max` chars, preferring a word boundary.
#[must_use]
pub fn clip(text: &str, max: usize) -> String {
    if text.len() <= max {
        return text.to_string();
    }
    // Try to cut at a word boundary.
    let cut = text[..max]
        .rfind(' ')
        .filter(|&i| i > max * 3 / 5)
        .unwrap_or(max);
    text[..cut].to_string()
}

/// Split text into non-empty, trimmed lines.
#[must_use]
pub fn non_empty_lines(text: &str) -> Vec<String> {
    text.lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .map(str::to_string)
        .collect()
}

/// Collapse `<skill_instructions name="X">...</skill_instructions>` blocks in a
/// list of lines into a single `[skill: X]` marker, deduped by name.
///
/// respondami's skill tag is `<skill_instructions name="X">` (not pi-vcc's
/// `<skill name="X">`), so the regex is adapted accordingly.
#[must_use]
pub fn collapse_skill_lines(lines: Vec<String>) -> Vec<String> {
    let mut result = Vec::new();
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut inside = false;
    for line in lines {
        if let Some(name) = line
            .trim_start()
            .strip_prefix("<skill_instructions")
            .and_then(|rest| rest.split('>').next())
            .and_then(|attr| attr.split("name=\"").nth(1))
            .and_then(|rest| rest.split('"').next())
        {
            inside = true;
            if seen.insert(name.to_string()) {
                result.push(format!("[skill: {name}]"));
            }
            continue;
        }
        if inside {
            if line.trim_start().starts_with("</skill_instructions") {
                inside = false;
            }
            continue;
        }
        result.push(line);
    }
    result
}

/// Collapse `<skill_instructions name="X">...</skill_instructions>` blocks in
/// raw text into a single `[skill: X]` marker.
#[must_use]
pub fn collapse_skill_text(text: &str) -> String {
    let mut out = String::new();
    let mut rest = text;
    while let Some(start) = rest.find("<skill_instructions") {
        out.push_str(&rest[..start]);
        let after = &rest[start..];
        // Find the closing tag.
        if let Some(close) = after.find("</skill_instructions") {
            let inner = &after[..close];
            let name = inner
                .split("name=\"")
                .nth(1)
                .and_then(|r| r.split('"').next())
                .unwrap_or("unknown");
            out.push_str(&format!("[skill: {name}]"));
            rest = &after[close + "</skill_instructions".len()..];
        } else {
            out.push_str(after);
            rest = "";
        }
    }
    out.push_str(rest);
    out
}

/// Parse a trailing `Exit code: N` line from a bash result.
///
/// The bash tool appends `Exit code: {n}` on success; a timeout returns an
/// error with `Command timed out after N seconds` and no exit code.
#[must_use]
pub fn parse_exit_code(result: &str) -> Option<i32> {
    result
        .lines()
        .rev()
        .find_map(|line| {
            line.trim()
                .strip_prefix("Exit code: ")
                .and_then(|n| n.parse::<i32>().ok())
        })
}

/// Normalize a single `AgentMessage`, pushing blocks into `out`.
///
/// `pending_bash` maps bash tool-call ids to the index of the placeholder
/// `Bash` block already emitted at the call position, so a `Tool` result can
/// fill in its output and exit code in place. Assistant `bash` tool calls are
/// recorded here and not emitted as `ToolCall` blocks (they surface as `Bash`).
fn normalize_message(
    msg: &AgentMessage,
    out: &mut Vec<VccBlock>,
    pending_bash: &mut HashMap<String, usize>,
) {
    match msg {
        AgentMessage::System { .. } => {}
        AgentMessage::User { content } => {
            let text = sanitize(content);
            if !text.trim().is_empty() {
                out.push(VccBlock::User { text });
            }
        }
        AgentMessage::Assistant { content: blocks, .. } => {
            for block in blocks {
                match block {
                    ContentBlock::Text { text } => {
                        let t = sanitize(text);
                        if !t.trim().is_empty() {
                            out.push(VccBlock::Assistant { text: t });
                        }
                    }
                    ContentBlock::Thinking { thinking } => {
                        // Design: include thinking as assistant context.
                        let t = sanitize(thinking);
                        if !t.trim().is_empty() {
                            out.push(VccBlock::Assistant { text: t });
                        }
                    }
                    ContentBlock::ToolCall { tool_call } => {
                        if tool_call.name.eq_ignore_ascii_case("bash") {
                            let command = tool_call
                                .arguments
                                .get("command")
                                .and_then(Value::as_str)
                                .unwrap_or("")
                                .to_string();
                            let index = out.len();
                            out.push(VccBlock::Bash {
                                command,
                                output: String::new(),
                                exit_code: None,
                            });
                            pending_bash.insert(tool_call.id.clone(), index);
                        } else {
                            out.push(VccBlock::ToolCall {
                                name: tool_call.name.clone(),
                                args: tool_call.arguments.clone(),
                            });
                        }
                    }
                }
            }
        }
        AgentMessage::Tool {
            tool_name,
            result,
            tool_call_id,
            ..
        } => {
            if tool_name.eq_ignore_ascii_case("bash") {
                let text = sanitize(result);
                let exit_code = parse_exit_code(result);
                if let Some(index) = pending_bash.remove(tool_call_id) {
                    let VccBlock::Bash { command, .. } = &out[index] else {
                        unreachable!("pending_bash indexes always point at Bash blocks");
                    };
                    out[index] = VccBlock::Bash {
                        command: command.clone(),
                        output: text,
                        exit_code,
                    };
                } else {
                    out.push(VccBlock::Bash {
                        command: String::new(),
                        output: text,
                        exit_code,
                    });
                }
            } else {
                let text = sanitize(result);
                out.push(VccBlock::ToolResult {
                    name: tool_name.clone(),
                    text,
                });
            }
        }
    }
}

/// Normalize a slice of session entries into blocks.
///
/// Only `Message` entries produce blocks; `Compaction`, `ModelChange`, `Custom`,
/// and `Session` entries are skipped. Bash tool calls get a placeholder `Bash`
/// block at the call position (empty output, no exit code) that is filled in
/// when the result arrives, so transcript order is preserved; a bash call with
/// no result (e.g. cancelled) keeps the empty placeholder.
#[must_use]
pub fn normalize_entries(entries: &[SessionEntry]) -> Vec<VccBlock> {
    let mut out = Vec::new();
    let mut pending_bash: HashMap<String, usize> = HashMap::new();
    for entry in entries {
        if let SessionEntry::Message { message, .. } = entry {
            normalize_message(message, &mut out, &mut pending_bash);
        }
    }
    out
}
