//! `SessionDisplayAdapter` — maps `SessionEntry` → `ChatMessage`.
//!
//! Single source of truth for session-to-display mapping.
//! Used by `session_select.rs` and any other code that needs to rebuild
//! the chat display from session data.

use crate::hooks::HookEvent;
use crate::session::{AgentMessage, SessionEntry};
use crate::tui::messages::tool_call::build_tool_call_variant;
use crate::tui::messages::{
    AssistantMessage, ChatMessage, HookMessage, ToolCallMessage, UserMessage,
};

/// Frame prefix of a persisted `PostToolUse` hook instruction block, as
/// written by the pre-simplification framer. Kept only to render *old*
/// sessions that already have these persisted; new sessions no longer
/// persist hook instructions as user messages.
const HOOK_INSTRUCTION_PREFIX: &str = "[Hook instruction — ";

/// Adapter that converts session entries into display-ready chat messages.
#[derive(Debug)]
pub struct SessionDisplayAdapter {
    /// Whether tool output is expanded by default.
    tool_output_expanded: bool,
}

impl SessionDisplayAdapter {
    /// Create a new adapter.
    #[must_use]
    pub fn new(tool_output_expanded: bool) -> Self {
        Self {
            tool_output_expanded,
        }
    }

    /// Build a list of `ChatMessage` from session entries.
    ///
    /// Skips system messages (they are injected by the agent loop).
    /// Skips empty assistant messages.
    #[must_use]
    pub fn build_messages(&self, entries: &[SessionEntry]) -> Vec<ChatMessage> {
        let mut messages = Vec::new();

        for entry in entries {
            if let SessionEntry::Message { message, .. } = entry {
                match message {
                    AgentMessage::User { content } => {
                        // Reconstruct hook instruction boxes (M3): a user
                        // message composed of `[Hook instruction — <name>]`
                        // framed blocks renders as hook boxes, not a user
                        // bubble. Malformed framing falls through to the
                        // user bubble (no error path).
                        match parse_hook_instruction_blocks(content) {
                            Some(blocks) => {
                                for (hook_name, body) in blocks {
                                    messages.push(ChatMessage::Hook(HookMessage {
                                        event: HookEvent::PostToolUse,
                                        hook_name,
                                        success: true,
                                        stdout: body,
                                        stderr: String::new(),
                                        tool_name: None,
                                    }));
                                }
                            }
                            None => {
                                messages.push(ChatMessage::User(UserMessage {
                                    content: content.clone(),
                                }));
                            }
                        }
                    }
                    AgentMessage::Assistant {
                        content: blocks, ..
                    } => {
                        use crate::session::ContentBlock;
                        // Decompose content blocks into visual entries.
                        // Thinking blocks become ThinkingMessage, text blocks become AssistantMessage,
                        // tool call blocks are handled separately (they appear as Tool entries in JSONL).
                        for block in blocks {
                            match block {
                                ContentBlock::Thinking { thinking } => {
                                    if !thinking.is_empty() {
                                        messages.push(ChatMessage::Thinking(
                                            crate::tui::messages::ThinkingMessage {
                                                reasoning: thinking.clone(),
                                            },
                                        ));
                                    }
                                }
                                ContentBlock::Text { text } => {
                                    if !text.is_empty() {
                                        messages.push(ChatMessage::Assistant(AssistantMessage {
                                            content: text.clone(),
                                        }));
                                    }
                                }
                                ContentBlock::ToolCall { .. } => {
                                    // Tool calls are stored as separate Tool entries in JSONL,
                                    // not embedded in the assistant message.
                                }
                            }
                        }
                    }
                    AgentMessage::Tool {
                        tool_name,
                        tool_arguments,
                        result,
                        ..
                    } => {
                        let variant = build_tool_call_variant(
                            tool_name,
                            result,
                            false, // tool call results from session are not errors
                            false, // cancelled flag is not persisted (no JSONL schema change)
                            Some(tool_arguments.clone()),
                            None, // no RTK original in session
                            self.tool_output_expanded,
                        );
                        messages.push(ChatMessage::ToolCall(ToolCallMessage { variant }));
                    }
                    AgentMessage::System { .. } => {
                        // System messages are skipped — they are injected by the agent loop
                    }
                }
            }
        }

        messages
    }
}

/// Parse `[Hook instruction — <name>]` framed blocks out of a user message.
///
/// The legacy framer emitted `\n\n[frame]\n<body>` per block; multiple blocks
/// were concatenated in registry order, so a message may carry several. This
/// only ever matches *old* sessions (new sessions no longer persist hook
/// instructions). Returns `Some(blocks)` only when the message is entirely
/// framed blocks; `None` otherwise (leading text, a frame without a closing
/// `]` on its header line, or an empty name), in which case the caller
/// renders a normal user bubble.
fn parse_hook_instruction_blocks(content: &str) -> Option<Vec<(String, String)>> {
    let first = content.trim_start_matches('\n');
    let first = first.strip_prefix(HOOK_INSTRUCTION_PREFIX)?;

    // Find every frame that starts a new block: a frame only counts when
    // preceded by the `\n\n` separator; an earlier occurrence inside a body
    // is body text.
    let mut starts: Vec<usize> = vec![0];
    let mut from = 0usize;
    while let Some(rel) = first[from..].find(HOOK_INSTRUCTION_PREFIX) {
        let pos = from + rel;
        if pos >= 2 && first[..pos].ends_with("\n\n") {
            starts.push(pos);
        }
        from = pos + HOOK_INSTRUCTION_PREFIX.len();
    }

    let mut blocks = Vec::new();
    for (i, &frame_pos) in starts.iter().enumerate() {
        // Content starts after the frame prefix (the first one was already
        // stripped); it runs to the next frame's `\n\n` separator (or the
        // end for the last block).
        let content_start = if i == 0 {
            0
        } else {
            frame_pos + HOOK_INSTRUCTION_PREFIX.len()
        };
        let content_end = match starts.get(i + 1) {
            Some(&next) => next - 2,
            None => first.len(),
        };
        let chunk = &first[content_start..content_end];
        let close = chunk.find(']')?;
        let name = &chunk[..close];
        if name.is_empty() || name.contains('\n') {
            return None;
        }
        let body = chunk[close + 1..]
            .strip_prefix('\n')
            .unwrap_or(&chunk[close + 1..]);
        blocks.push((name.to_string(), body.to_string()));
    }
    Some(blocks)
}
