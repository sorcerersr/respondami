//! Unknown tool call rendering — fallback for unrecognized tools.
//!
//! Stores tool name and raw result for display when no specific renderer exists.
//! Implements `ToolCallRender` for chat display.

use super::ToolCallRender;

/// Fallback for unrecognized tool calls.
#[derive(Debug, Clone)]
pub struct UnknownToolCall {
    pub tool_name: String,
    pub result: String,
    pub has_error: bool,
    pub expanded: bool,
}

impl ToolCallRender for UnknownToolCall {
    fn content_lines(&self) -> Vec<String> {
        if self.result.is_empty() {
            return vec![];
        }
        self.result.lines().map(std::string::ToString::to_string).collect()
    }

    fn is_expanded(&self) -> bool {
        self.expanded
    }
}
