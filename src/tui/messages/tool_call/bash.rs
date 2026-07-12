//! Bash tool call rendering.
//!
//! Parses bash tool call results into structured data: command, output, exit code,
//! error status, and RTK original command. Implements `ToolCallRender` for chat display.

use super::ToolCallRender;

/// Parsed data for a `bash` tool call.
#[derive(Debug, Clone)]
pub struct BashToolCall {
    pub command: String,
    pub output: String,
    pub exit_code: Option<i32>,
    pub has_error: bool,
    pub rtk_original: Option<String>,
    pub expanded: bool,
}

impl BashToolCall {
    #[must_use]
    pub fn from_args(result: &str, has_error: bool, tool_args: Option<&serde_json::Value>, rtk_original: Option<String>, expanded: bool) -> Self {
        let command = tool_args
            .and_then(|a| a.get("command"))
            .and_then(|v| v.as_str())
            .unwrap_or("unknown")
            .to_string();

        Self {
            command,
            output: result.to_string(),
            exit_code: None,
            has_error,
            rtk_original,
            expanded,
        }
    }
}

impl ToolCallRender for BashToolCall {
    fn content_lines(&self) -> Vec<String> {
        if self.has_error {
            return self.output.lines().map(std::string::ToString::to_string).collect();
        }
        if self.output.is_empty() {
            return vec![];
        }
        self.output.lines().map(std::string::ToString::to_string).collect()
    }

    fn is_expanded(&self) -> bool {
        self.expanded
    }
}
