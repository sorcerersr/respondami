//! `SessionDisplayAdapter` — maps `SessionEntry` → `ChatMessage`.
//!
//! Single source of truth for session-to-display mapping.
//! Used by `session_select.rs` and any other code that needs to rebuild
//! the chat display from session data.

use crate::session::{AgentMessage, SessionEntry};
use crate::tui::messages::{
    AssistantMessage, ChatMessage, ToolCallMessage, UserMessage,
};
use crate::tui::messages::tool_call::build_tool_call_variant;

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
                        messages.push(ChatMessage::User(UserMessage {
                            content: content.clone(),
                        }));
                    }
                    AgentMessage::Assistant { content: blocks, .. } => {
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
