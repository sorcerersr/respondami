//! Session data types — JSONL entry formats and serialization.
//!
//! Defines `SessionEntry` (tagged enum for JSONL persistence), `AgentMessage`,
//! `ToolCall`, `Usage`, `RequestTokenUsage`, and `TokenRateEntry`. All types
//! derive `Serialize`/`Deserialize` for JSONL round-tripping.

use serde::{Deserialize, Serialize};

/// A session entry stored in JSONL format.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum SessionEntry {
    #[serde(rename = "session")]
    Session {
        version: u32,
        id: String,
        timestamp: String,
        cwd: String,
        model: String,
        context_window: u32,
        /// Skills that have been activated in this session.
        #[serde(default)]
        activated_skills: Vec<String>,
    },
    #[serde(rename = "message")]
    Message {
        id: String,
        parent_id: Option<String>,
        timestamp: String,
        message: AgentMessage,
    },
    #[serde(rename = "compaction")]
    Compaction {
        id: String,
        parent_id: Option<String>,
        timestamp: String,
        summary: String,
        first_kept_entry_id: String,
        tokens_before: u32,
    },
    #[serde(rename = "model_change")]
    ModelChange {
        id: String,
        parent_id: Option<String>,
        timestamp: String,
        model: String,
        context_window: u32,
    },
    #[serde(rename = "custom")]
    Custom {
        custom_type: String,
        data: serde_json::Value,
    },
}

/// A structured content block within an assistant message.
///
/// Replaces the flat `content`/`reasoning`/`tool_calls` model with a unified
/// array. Each block is a first-class element that is persisted in JSONL,
/// restored to LLM context on resume, and included in compaction summaries.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ContentBlock {
    /// Visible text content.
    Text { text: String },
    /// Thinking/reasoning content (hidden by default in TUI).
    Thinking { thinking: String },
    /// A tool call.
    ToolCall { tool_call: ToolCall },
}

/// A message within the agent conversation.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "role")]
pub enum AgentMessage {
    #[serde(rename = "system")]
    System { content: String },
    #[serde(rename = "user")]
    User { content: String },
    #[serde(rename = "assistant")]
    Assistant {
        /// Structured content blocks (text, thinking, tool calls).
        content: Vec<ContentBlock>,
        #[serde(skip_serializing_if = "Option::is_none")]
        usage: Option<Usage>,
    },
    #[serde(rename = "tool")]
    Tool {
        tool_call_id: String,
        tool_name: String,
        tool_arguments: serde_json::Value,
        result: String,
    },
}

/// A tool call made by the assistant.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCall {
    pub id: String,
    pub name: String,
    pub arguments: serde_json::Value,
}

/// Token usage from an LLM response.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Usage {
    pub prompt_tokens: u32,
    pub completion_tokens: u32,
    pub total_tokens: u32,
}

/// Token usage for a single request (input = prompt, output = completion).
///
/// Supports the max+delta accumulation pattern from zed:
/// - Within a single request: takes the max of each field (deduplicates
///   repeated SSE usage events)
/// - Between requests: only adds the delta to cumulative totals
///
/// The `estimated` flag is set to `true` when using fallback estimation
/// (len/4 heuristic) during streaming, and cleared when real Usage events
/// arrive.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize)]
pub struct RequestTokenUsage {
    pub input_tokens: u32,
    pub output_tokens: u32,
    #[serde(default)]
    pub estimated: bool,
}

impl RequestTokenUsage {
    #[must_use]
    pub fn total(&self) -> u32 {
        self.input_tokens + self.output_tokens
    }

    /// Compute the delta between two usage snapshots.
    /// Returns zero for fields where the new value is smaller (safety).
    #[must_use]
    pub fn delta(&self, previous: &Self) -> Self {
        Self {
            input_tokens: self.input_tokens.saturating_sub(previous.input_tokens),
            output_tokens: self.output_tokens.saturating_sub(previous.output_tokens),
            estimated: false,
        }
    }
}

/// Per-turn token rate snapshot saved to session JSONL.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TokenRateEntry {
    pub tokens: u32,
    pub seconds: f64,
}

impl TokenRateEntry {
    #[must_use]
    pub fn new(tokens: u32, seconds: f64) -> Self {
        Self { tokens, seconds }
    }
}

impl SessionEntry {
    /// Create a new session header entry.
    #[must_use]
    pub fn new_session(id: String, timestamp: String, cwd: String, model: String, context_window: u32) -> Self {
        SessionEntry::Session {
            version: 1,
            id,
            timestamp,
            cwd,
            model,
            context_window,
            activated_skills: Vec::new(),
        }
    }

    /// Create a new message entry.
    #[must_use]
    pub fn new_message(parent_id: Option<String>, message: AgentMessage) -> Self {
        SessionEntry::Message {
            id: uuid::Uuid::new_v4().to_string(),
            parent_id,
            timestamp: chrono::Utc::now().to_rfc3339(),
            message,
        }
    }

    /// Create a new compaction entry.
    #[must_use]
    pub fn new_compaction(
        parent_id: Option<String>,
        summary: String,
        first_kept_entry_id: String,
        tokens_before: u32,
    ) -> Self {
        SessionEntry::Compaction {
            id: uuid::Uuid::new_v4().to_string(),
            parent_id,
            timestamp: chrono::Utc::now().to_rfc3339(),
            summary,
            first_kept_entry_id,
            tokens_before,
        }
    }

    /// Create a new model change entry.
    #[must_use]
    pub fn new_model_change(parent_id: Option<String>, model: String, context_window: u32) -> Self {
        SessionEntry::ModelChange {
            id: uuid::Uuid::new_v4().to_string(),
            parent_id,
            timestamp: chrono::Utc::now().to_rfc3339(),
            model,
            context_window,
        }
    }

    /// Create a custom entry (e.g. token-rate snapshots).
    #[must_use]
    pub fn new_custom(custom_type: String, data: serde_json::Value) -> Self {
        SessionEntry::Custom { custom_type, data }
    }

    /// Get the entry ID.
    #[must_use]
    pub fn id(&self) -> String {
        match self {
            SessionEntry::Session { id, .. }
            | SessionEntry::Message { id, .. }
            | SessionEntry::Compaction { id, .. }
            | SessionEntry::ModelChange { id, .. } => id.clone(),
            SessionEntry::Custom { .. } => String::new(),
        }
    }

    /// Get the parent ID.
    #[must_use]
    pub fn parent_id(&self) -> Option<String> {
        match self {
            SessionEntry::Session { .. } | SessionEntry::Custom { .. } => None,
            SessionEntry::Message { parent_id, .. }
            | SessionEntry::Compaction { parent_id, .. }
            | SessionEntry::ModelChange { parent_id, .. } => parent_id.clone(),
        }
    }
}

impl AgentMessage {
    /// Create a system message.
    #[must_use]
    pub fn system(content: String) -> Self {
        AgentMessage::System { content }
    }

    /// Create a user message.
    #[must_use]
    pub fn user(content: String) -> Self {
        AgentMessage::User { content }
    }

    /// Create an assistant message from structured content blocks.
    #[must_use]
    pub fn assistant_with_blocks(content: Vec<ContentBlock>, usage: Option<Usage>) -> Self {
        AgentMessage::Assistant { content, usage }
    }

    /// Create an assistant message from legacy separate fields.
    ///
    /// Converts `content`/`reasoning`/`tool_calls` into `Vec<ContentBlock>`.
    /// Thinking blocks come before text blocks to match streaming order.
    #[must_use]
    pub fn assistant(content: String, reasoning: String, tool_calls: Vec<ToolCall>, usage: Option<Usage>) -> Self {
        let mut blocks = Vec::new();
        if !reasoning.is_empty() {
            blocks.push(ContentBlock::Thinking { thinking: reasoning });
        }
        if !content.is_empty() {
            blocks.push(ContentBlock::Text { text: content });
        }
        for tc in tool_calls {
            blocks.push(ContentBlock::ToolCall { tool_call: tc });
        }
        AgentMessage::Assistant { content: blocks, usage }
    }

    /// Create a tool result message.
    #[must_use]
    pub fn tool(tool_call_id: String, tool_name: String, tool_arguments: serde_json::Value, result: String) -> Self {
        AgentMessage::Tool {
            tool_call_id,
            tool_name,
            tool_arguments,
            result,
        }
    }

    /// Get the content string for display.
    /// For assistant messages, returns concatenated text blocks.
    #[must_use]
    pub fn content(&self) -> &str {
        match self {
            AgentMessage::System { content } => content,
            AgentMessage::User { content } => content,
            AgentMessage::Tool { result, .. } => result,
            AgentMessage::Assistant { content: blocks, .. } => {
                // Return first text block's content, or empty string
                static EMPTY: &str = "";
                blocks.iter().find_map(|b| {
                    if let ContentBlock::Text { text } = b {
                        Some(text.as_str())
                    } else {
                        None
                    }
                }).unwrap_or(EMPTY)
            }
        }
    }

    /// Concatenate all text blocks.
    #[must_use]
    pub fn text(&self) -> String {
        match self {
            AgentMessage::Assistant { content: blocks, .. } => {
                blocks.iter()
                    .filter_map(|b| {
                        if let ContentBlock::Text { text } = b {
                            Some(text.as_str())
                        } else {
                            None
                        }
                    })
                    .collect::<Vec<_>>()
                    .join("\n")
            }
            AgentMessage::System { content } | AgentMessage::User { content } => content.clone(),
            AgentMessage::Tool { result, .. } => result.clone(),
        }
    }

    /// Concatenate all thinking blocks.
    #[must_use]
    pub fn thinking(&self) -> String {
        match self {
            AgentMessage::Assistant { content: blocks, .. } => {
                blocks.iter()
                    .filter_map(|b| {
                        if let ContentBlock::Thinking { thinking } = b {
                            Some(thinking.as_str())
                        } else {
                            None
                        }
                    })
                    .collect::<Vec<_>>()
                    .join("\n")
            }
            _ => String::new(),
        }
    }

    /// Extract all tool calls from content blocks.
    #[must_use]
    pub fn tool_calls(&self) -> Vec<&ToolCall> {
        match self {
            AgentMessage::Assistant { content: blocks, .. } => {
                blocks.iter()
                    .filter_map(|b| {
                        if let ContentBlock::ToolCall { tool_call } = b {
                            Some(tool_call)
                        } else {
                            None
                        }
                    })
                    .collect()
            }
            _ => Vec::new(),
        }
    }

    /// Check if this assistant message has any thinking blocks.
    #[must_use]
    pub fn has_thinking(&self) -> bool {
        match self {
            AgentMessage::Assistant { content: blocks, .. } => {
                blocks.iter().any(|b| matches!(b, ContentBlock::Thinking { .. }))
            }
            _ => false,
        }
    }

    /// Check if this assistant message has any tool call blocks.
    #[must_use]
    pub fn has_tool_calls(&self) -> bool {
        match self {
            AgentMessage::Assistant { content: blocks, .. } => {
                blocks.iter().any(|b| matches!(b, ContentBlock::ToolCall { .. }))
            }
            _ => false,
        }
    }
}
