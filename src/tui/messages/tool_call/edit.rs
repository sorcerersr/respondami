//! Edit tool call rendering.
//!
//! Parses edit tool call results into structured data: path, list of edits (oldText/newText),
//! and error status. Implements `ToolCallRender` for diff-style chat display.

use super::ToolCallRender;

/// A single text replacement edit.
#[derive(Debug, Clone)]
pub struct EditDiff {
    pub old_text: String,
    pub new_text: String,
}

/// Parsed data for an `edit` tool call.
#[derive(Debug, Clone)]
pub struct EditToolCall {
    pub path: String,
    pub edits: Vec<EditDiff>,
    pub has_error: bool,
    pub expanded: bool,
}

impl EditToolCall {
    #[must_use]
    pub fn from_args(_result: &str, has_error: bool, tool_args: Option<&serde_json::Value>, expanded: bool) -> Self {
        if has_error {
            return Self {
                path: tool_args.and_then(|a| a.get("path").and_then(|v| v.as_str())).unwrap_or("unknown").to_string(),
                edits: Vec::new(),
                has_error: true,
                expanded,
            };
        }

        let path = tool_args.and_then(|a| a.get("path").and_then(|v| v.as_str())).unwrap_or("unknown").to_string();
        let edits = tool_args
            .and_then(|a| a.get("edits"))
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|e| {
                        let old_text = e.get("oldText")?.as_str()?.to_string();
                        let new_text = e.get("newText")?.as_str()?.to_string();
                        Some(EditDiff { old_text, new_text })
                    })
                    .collect()
            })
            .unwrap_or_default();

        Self { path, edits, has_error, expanded }
    }
}

impl ToolCallRender for EditToolCall {
    fn content_lines(&self) -> Vec<String> {
        if self.has_error {
            return vec![format!("Failed to edit {}", self.path)];
        }
        let mut lines = Vec::new();
        for diff in &self.edits {
            for line in diff.old_text.lines() {
                lines.push(format!("- {line}"));
            }
            for line in diff.new_text.lines() {
                lines.push(format!("+ {line}"));
            }
        }
        lines
    }

    fn is_expanded(&self) -> bool {
        self.expanded
    }
}
