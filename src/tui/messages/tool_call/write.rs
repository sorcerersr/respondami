//! Write tool call rendering.
//!
//! Parses write tool call results into structured data: path and content.
//! Implements `ToolCallRender` for chat display.

use super::ToolCallRender;

/// Parsed data for a `write` tool call.
#[derive(Debug, Clone)]
pub struct WriteToolCall {
    pub path: String,
    pub content: String,
    pub has_error: bool,
    pub expanded: bool,
}

impl WriteToolCall {
    #[must_use]
    pub fn from_args(_result: &str, has_error: bool, tool_args: Option<&serde_json::Value>, expanded: bool) -> Self {
        if has_error {
            return Self {
                path: tool_args.and_then(|a| a.get("path").and_then(|v| v.as_str())).unwrap_or("unknown").to_string(),
                content: String::new(),
                has_error: true,
                expanded,
            };
        }

        let path = tool_args.and_then(|a| a.get("path").and_then(|v| v.as_str())).unwrap_or("unknown").to_string();
        let content = tool_args
            .and_then(|a| a.get("content"))
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();

        Self { path, content, has_error: false, expanded }
    }
}

impl ToolCallRender for WriteToolCall {
    fn content_lines(&self) -> Vec<String> {
        if self.has_error {
            return vec![format!("Failed to write {}", self.path)];
        }
        if self.content.is_empty() {
            return vec![];
        }
        self.content.lines().map(std::string::ToString::to_string).collect()
    }

    fn is_expanded(&self) -> bool {
        self.expanded
    }
}
