//! Agent streaming state — in-flight tokens, pending tool calls, token tracking.
//!
//! Tracks `streaming_content` (accumulated token text), `pending_tool_calls` and
//! `pending_tool_call_ids` (tool calls being accumulated during streaming), and
//! `TokenRateTracker` for instant/average token rate display.

use std::collections::HashMap;

use super::agent_event::PartialToolCall;
use crate::context::TokenRateTracker;

/// Agent streaming state: in-flight tokens, pending tool calls, token tracking.
#[derive(Debug)]
pub struct AgentState {
    pub streaming_content: String,
    pub pending_tool_calls: Vec<PartialToolCall>,
    pub pending_tool_call_ids: HashMap<String, usize>,
    pub tracker: TokenRateTracker,
    /// The user message that triggered the current streaming turn.
    /// Used for fallback token estimation during streaming.
    pub current_user_message: Option<String>,
}

impl AgentState {
    #[must_use]
    pub fn new() -> Self {
        Self {
            streaming_content: String::new(),
            pending_tool_calls: Vec::new(),
            pending_tool_call_ids: HashMap::new(),
            tracker: TokenRateTracker::new(),
            current_user_message: None,
        }
    }
}

impl Default for AgentState {
    fn default() -> Self {
        Self::new()
    }
}
