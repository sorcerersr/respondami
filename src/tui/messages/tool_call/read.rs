//! Read tool call rendering.
//!
//! Parses read tool call results into structured data: path, offset, limit,
//! total lines, and content. Implements `ToolCallRender` for chat display.

use super::ToolCallRender;

/// Parsed data for a `read` tool call.
#[derive(Debug, Clone)]
pub struct ReadToolCall {
    pub path: String,
    pub offset: Option<u64>,
    pub limit: Option<u64>,
    pub total_lines: Option<u32>,
    pub content: String,
    pub has_error: bool,
    pub expanded: bool,
}

impl ReadToolCall {
    /// Parse tool args and result to extract structured data.
    #[must_use]
    pub fn from_args(result: &str, has_error: bool, tool_args: Option<&serde_json::Value>, expanded: bool) -> Self {
        if has_error {
            return Self {
                path: tool_args.and_then(|a| a.get("path").and_then(|v| v.as_str())).unwrap_or("unknown").to_string(),
                offset: None,
                limit: None,
                total_lines: None,
                content: result.to_string(),
                has_error: true,
                expanded,
            };
        }

        let path = tool_args.and_then(|a| a.get("path").and_then(|v| v.as_str())).unwrap_or("unknown").to_string();
        let offset = tool_args.and_then(|a| a.get("offset").and_then(serde_json::Value::as_u64));
        let limit = tool_args.and_then(|a| a.get("limit").and_then(serde_json::Value::as_u64));

        let total_lines = if let Some(first_line) = result.lines().next() {
            if let Some(stripped) = first_line.strip_prefix("Showing lines ") {
                if let Some(rest) = stripped.strip_suffix(':') {
                    let parts: Vec<&str> = rest.split_whitespace().collect();
                    if parts.len() >= 3 {
                        parts[2].parse().ok()
                    } else {
                        None
                    }
                } else if let Some(stripped) = first_line.strip_prefix("Showing first ") {
                    if let Some(rest) = stripped.strip_suffix(':') {
                        let parts: Vec<&str> = rest.split_whitespace().collect();
                        if parts.len() >= 3 {
                            parts[2].parse().ok()
                        } else {
                            None
                        }
                    } else {
                        None
                    }
                } else if let Some(total) = first_line.strip_suffix(" lines:") {
                    total.parse().ok()
                } else {
                    None
                }
            } else {
                None
            }
        } else {
            None
        };

        Self {
            path,
            offset,
            limit,
            total_lines,
            content: result.to_string(),
            has_error: false,
            expanded,
        }
    }

    #[must_use]
    pub fn range_meta(&self) -> Option<String> {
        match (self.offset, self.limit) {
            (Some(offset), Some(limit)) if limit > 0 => {
                Some(format!(" :{}-{}", offset, offset + limit - 1))
            }
            _ => None,
        }
    }
}

impl ToolCallRender for ReadToolCall {
    fn content_lines(&self) -> Vec<String> {
        if self.has_error {
            return self.content.lines().map(std::string::ToString::to_string).collect();
        }
        self.content.lines().skip_while(|l| {
            l.starts_with("Showing lines ") || l.starts_with("Showing first ") || l.ends_with(" lines:")
        }).map(std::string::ToString::to_string).collect()
    }

    fn is_expanded(&self) -> bool {
        self.expanded
    }
}
