//! VCC brief transcript rendering.
//!
//! `build_brief_sections` turns blocks into `[user]` / `[assistant]` sections;
//! `stringify_brief` renders them to text. User text is truncated to a word
//! budget; assistant text uses head+tail word truncation; tool calls become
//! one-liners; bash commands are compressed. Consecutive identical tool lines
//! are collapsed and tool calls per turn are capped.
//!
//! Ported from pi-vcc's `brief.ts`. There are no `#N` refs (no recall).

use serde_json::Value;

use super::bash::compress_bash;
use super::normalize::{clip, collapse_skill_text};
use super::sections::extract_path;
use super::normalize::VccBlock;

const TRUNCATE_USER: usize = 256;
const ASSISTANT_HEAD_WORDS: usize = 80;
const ASSISTANT_TAIL_WORDS: usize = 120;
const SEGMENT_CLOSING_ASSISTANT_HEAD_WORDS: usize = 120;
const SEGMENT_CLOSING_ASSISTANT_TAIL_WORDS: usize = 120;
const TOOL_CALLS_PER_TURN: usize = 8;

/// Common stop words that don't count toward a word budget.
const STOP_WORDS: &[&str] = &[
    "a", "an", "the", "is", "are", "was", "were", "be", "been", "being", "have", "has", "had",
    "do", "does", "did", "will", "would", "could", "should", "may", "might", "shall", "can",
    "need", "must", "to", "of", "in", "for", "on", "with", "at", "by", "from", "as", "into",
    "through", "during", "before", "after", "above", "below", "between", "under", "over", "and",
    "but", "or", "nor", "not", "so", "yet", "both", "either", "neither", "each", "every", "all",
    "any", "few", "more", "most", "other", "some", "such", "no", "that", "this", "these",
    "those", "it", "its", "i", "me", "my", "we", "our", "you", "your", "he", "him", "his", "she",
    "her", "they", "them", "their", "who", "which", "what", "if", "then", "than", "when", "where",
    "how", "just", "also",
];

fn is_stop_word(word: &str) -> bool {
    STOP_WORDS.contains(&word.to_lowercase().as_str())
}

/// Normalize whitespace for a word budget.
fn normalize_for_budget(text: &str) -> String {
    let flat = text.replace("\r\n", "\n").replace('\r', "\n");
    let mut out = String::with_capacity(flat.len());
    let mut space = false;
    let mut blank_run = 0usize;
    for c in flat.chars() {
        if c == '\n' {
            if blank_run == 0 {
                out.push('\n');
            }
            blank_run += 1;
            space = false;
        } else if c.is_whitespace() {
            if !space && !out.ends_with('\n') {
                out.push(' ');
                space = true;
            }
        } else {
            out.push(c);
            space = false;
            blank_run = 0;
        }
    }
    out.trim().to_string()
}

/// Truncate to the first `limit` significant (non-stop) words.
fn truncate_tokens(text: &str, limit: usize) -> String {
    let flat = normalize_for_budget(text);
    let mut count = 0usize;
    let mut last_end = 0usize;
    for word in flat.split(' ') {
        if !word.is_empty() && !is_stop_word(word) {
            count += 1;
            if count > limit {
                return format!("{}...(truncated)", flat[..last_end].trim_end());
            }
        }
        last_end += word.len() + 1;
    }
    flat
}

/// Truncate to `head_limit` + `tail_limit` significant words, keeping both ends.
fn truncate_tokens_head_tail(text: &str, head_limit: usize, tail_limit: usize) -> String {
    let flat = normalize_for_budget(text);
    if head_limit == 0 || tail_limit == 0 {
        return flat;
    }
    let words: Vec<&str> = flat.split(' ').filter(|w| !w.is_empty()).collect();
    let significant: Vec<&str> = words.iter().copied().filter(|w| !is_stop_word(w)).collect();
    if significant.len() <= head_limit + tail_limit {
        return flat;
    }
    // Find the byte offset of the head_limit-th significant word's end.
    let mut seen = 0usize;
    let mut head_end = 0usize;
    let mut offset = 0usize;
    for w in &words {
        if !is_stop_word(w) {
            seen += 1;
            if seen == head_limit {
                head_end = offset + w.len();
                break;
            }
        }
        offset += w.len() + 1;
    }
    // Find the byte offset of the (len - tail_limit)-th significant word's start.
    let total = significant.len();
    let tail_start_sig = total - tail_limit;
    let mut seen2 = 0usize;
    let mut tail_start = flat.len();
    offset = 0;
    for w in &words {
        if !is_stop_word(w) {
            seen2 += 1;
            if seen2 == tail_start_sig {
                tail_start = offset;
                break;
            }
        }
        offset += w.len() + 1;
    }
    format!(
        "{}\n...(middle truncated)...\n{}",
        flat[..head_end].trim_end(),
        flat[tail_start..].trim_start()
    )
}

/// A brief section: a header and its content lines.
#[derive(Debug, Default)]
pub struct BriefLine {
    pub header: String,
    pub lines: Vec<String>,
}

/// A one-line tool-call summary.
fn tool_one_liner(name: &str, args: &Value) -> String {
    // File-path tools.
    for key in ["file_path", "path", "filePath", "file"] {
        if let Some(p) = args.get(key).and_then(Value::as_str) {
            return format!("* {name} \"{p}\"");
        }
    }
    if let Some(p) = extract_path(args) {
        return format!("* {name} \"{p}\"");
    }
    if name.eq_ignore_ascii_case("bash") {
        let raw = args
            .get("command")
            .and_then(Value::as_str)
            .or_else(|| args.get("description").and_then(Value::as_str))
            .unwrap_or("");
        let cmd = compress_bash(raw);
        return format!("* {name} \"{cmd}\"");
    }
    if let Some(q) = args.get("query").and_then(Value::as_str) {
        return format!("* {name} \"{}\"", clip(q, 60));
    }
    format!("* {name}")
}

fn is_segment_closing_assistant(blocks: &[VccBlock], index: usize) -> bool {
    if blocks.get(index).is_none_or(|b| !matches!(b, VccBlock::Assistant { .. })) {
        return false;
    }
    for b in &blocks[index + 1..] {
        if matches!(b, VccBlock::ToolResult { .. }) {
            continue;
        }
        return matches!(b, VccBlock::User { .. });
    }
    true
}

/// Build brief sections from blocks.
#[must_use]
pub fn build_brief_sections(blocks: &[VccBlock]) -> Vec<BriefLine> {
    let mut sections: Vec<BriefLine> = Vec::new();
    let mut last_header = String::new();

    let push = |header: &str, line: String, sections: &mut Vec<BriefLine>, last_header: &mut String| {
        if header == last_header && !sections.is_empty() {
            sections.last_mut().unwrap().lines.push(line);
            return;
        }
        sections.push(BriefLine {
            header: header.to_string(),
            lines: vec![line],
        });
        *last_header = header.to_string();
    };

    for (block_index, b) in blocks.iter().enumerate() {
        match b {
            VccBlock::User { text } => {
                if text.trim().is_empty() {
                    continue;
                }
                let text = truncate_tokens(&collapse_skill_text(text), TRUNCATE_USER);
                if !text.is_empty() {
                    for line in text.lines() {
                        push("[user]", line.to_string(), &mut sections, &mut last_header);
                    }
                }
                last_header = "[user]".to_string();
            }
            VccBlock::Bash { command, .. } => {
                let cmd = compress_bash(command);
                if !cmd.is_empty() {
                    push("[user]", format!("$ {cmd}"), &mut sections, &mut last_header);
                }
                last_header = "[user]".to_string();
            }
            VccBlock::Assistant { text } => {
                let raw = text.clone();
                let text = if is_segment_closing_assistant(blocks, block_index) {
                    truncate_tokens_head_tail(
                        &raw,
                        SEGMENT_CLOSING_ASSISTANT_HEAD_WORDS,
                        SEGMENT_CLOSING_ASSISTANT_TAIL_WORDS,
                    )
                } else {
                    truncate_tokens_head_tail(&raw, ASSISTANT_HEAD_WORDS, ASSISTANT_TAIL_WORDS)
                };
                if !text.is_empty() {
                    for line in text.lines() {
                        push("[assistant]", line.to_string(), &mut sections, &mut last_header);
                    }
                }
            }
            VccBlock::ToolCall { name, args } => {
                if name.trim().is_empty() {
                    continue;
                }
                push(
                    "[assistant]",
                    tool_one_liner(name, args),
                    &mut sections,
                    &mut last_header,
                );
            }
            VccBlock::ToolResult { .. } => {
                // Tool result bodies are intentionally omitted from briefs.
            }
        }
    }

    // Cap tool calls per [assistant] turn — keep the tail.
    for sec in &mut sections {
        if sec.header != "[assistant]" {
            continue;
        }
        let tool_idxs: Vec<usize> = sec
            .lines
            .iter()
            .enumerate()
            .filter_map(|(i, l)| l.starts_with("* ").then_some(i))
            .collect();
        if tool_idxs.len() <= TOOL_CALLS_PER_TURN {
            continue;
        }
        let drop_count = tool_idxs.len() - TOOL_CALLS_PER_TURN;
        let drop_set: std::collections::HashSet<usize> = tool_idxs[..drop_count].iter().copied().collect();
        let first_kept = tool_idxs[drop_count];
        let mut next: Vec<String> = Vec::new();
        let mut inserted = false;
        for (i, line) in sec.lines.iter().enumerate() {
            if drop_set.contains(&i) {
                continue;
            }
            if !inserted && i == first_kept {
                next.push(format!("* ({drop_count} earlier tool-call entries omitted)"));
                inserted = true;
            }
            next.push(line.clone());
        }
        sec.lines = next;
    }

    sections
}

/// Stringify brief sections into text.
#[must_use]
pub fn stringify_brief(sections: &[BriefLine]) -> String {
    let mut out: Vec<String> = Vec::new();
    for (i, sec) in sections.iter().enumerate() {
        if i > 0 {
            let prev = &sections[i - 1];
            let prev_is_tools = prev.header == "[assistant]"
                && !prev.lines.is_empty()
                && prev.lines.iter().all(|l| l.starts_with("* "));
            let cur_is_tools = sec.header == "[assistant]"
                && !sec.lines.is_empty()
                && sec.lines.iter().all(|l| l.starts_with("* "));
            if !(prev_is_tools && cur_is_tools) {
                out.push(String::new());
            }
        }
        out.push(sec.header.clone());
        out.extend(sec.lines.iter().cloned());
    }
    out.join("\n")
}

/// Convenience: build sections from blocks and stringify to text.
#[must_use]
pub fn compile_brief(blocks: &[VccBlock]) -> String {
    stringify_brief(&build_brief_sections(blocks))
}

/// Cap a brief to `max_lines`, keeping the tail and avoiding a mid-section cut.
#[must_use]
pub fn cap_brief(text: &str, max_lines: usize) -> String {
    if text.is_empty() || max_lines == 0 {
        return String::new();
    }
    let lines: Vec<&str> = text.lines().collect();
    if lines.len() <= max_lines {
        return text.to_string();
    }
    let kept = &lines[lines.len() - max_lines..];
    let first_header = kept.iter().position(|l| {
        let t = l.trim_start();
        t.starts_with('[') && t.contains(']')
    });
    let clean = match first_header {
        Some(p) if p > 0 => &kept[p..],
        _ => kept,
    };
    let omitted = lines.len() - clean.len();
    format!("...({omitted} earlier lines omitted)\n\n{}", clean.join("\n"))
}

/// Cap a brief to the default line budget.
#[must_use]
pub fn cap_brief_default(text: &str) -> String {
    cap_brief(text, super::BRIEF_MAX_LINES)
}
