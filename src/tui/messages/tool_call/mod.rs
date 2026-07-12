//! Tool call message rendering for the chat viewport.
//!
//! Defines `ToolCallMessage`, `ToolCallVariant`, and per-tool call types
//! (Bash, Read, Write, Edit, Unknown) with `FilledHeaderBar` rendering.
//!
//! Rust guideline compliant 2026-02-21

use std::time::{Duration, Instant};

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::text::{Line, Span};

use ratatui_md::HeightAware;
use ratatui_widgets::FilledHeaderBar;
use crate::tui::theme::Theme;

mod bash;
mod edit;
mod read;
mod unknown;
mod write;

#[cfg(test)]
mod mod_tests;

#[doc(inline)]
pub use bash::BashToolCall;
#[doc(inline)]
pub use edit::EditToolCall;
#[doc(inline)]
pub use read::ReadToolCall;
#[doc(inline)]
pub use unknown::UnknownToolCall;
#[doc(inline)]
pub use write::WriteToolCall;

/// Maximum number of content lines to show before truncating.
pub const MAX_CONTENT_LINES: usize = 10;

/// Duration of the stretch animation for new tool calls.
pub const ANIMATION_DURATION: Duration = Duration::from_millis(400);

/// A tool call result message.
#[derive(Debug, Clone)]
pub struct ToolCallMessage {
    pub variant: ToolCallVariant,
}

/// Specific tool call type with parsed, strongly-typed data.
#[derive(Debug, Clone)]
pub enum ToolCallVariant {
    /// Tool call is executing — show pending indicator + streamed output.
    Pending {
        tool_name: String,
        tool_args: serde_json::Value,
        output: String,
        animation_start: Option<Instant>,
        effect_added: bool,
    },
    Read(ReadToolCall),
    Write(WriteToolCall),
    Edit(EditToolCall),
    Bash(BashToolCall),
    Unknown(UnknownToolCall),
}

/// Build content lines for a non-pending tool call.
///
/// When content ≤ `MAX_CONTENT_LINES`: all content lines + meta line (if any).
/// When content > `MAX_CONTENT_LINES` and expanded: summary line + last 10 lines + meta line.
/// When content > `MAX_CONTENT_LINES` and not expanded: returns None, caller uses `FilledHeaderBar::collapsed(true)`.
fn build_content_lines(
    content_lines: &[String],
    has_error: bool,
    expanded: bool,
    meta: Option<String>,
    theme: &Theme,
) -> Option<Vec<Line<'static>>> {
    // Error: always show all lines (no truncation)
    if has_error {
        let mut lines: Vec<Line<'static>> = content_lines
            .iter()
            .map(|line| Line::from(Span::styled(line.clone(), theme.tool_error_content_style())))
            .collect();
        if let Some(meta) = meta {
            lines.push(Line::from(Span::styled(meta, theme.tool_meta_style())));
        }
        return Some(lines);
    }

    if content_lines.is_empty() {
        let mut lines = vec![Line::from(Span::styled(
            "(no output)".to_string(),
            theme.tool_meta_style(),
        ))];
        if let Some(meta) = meta {
            lines.push(Line::from(Span::styled(meta, theme.tool_meta_style())));
        }
        return Some(lines);
    }

    let total = content_lines.len();
    if total <= MAX_CONTENT_LINES {
        // Fits within limit: show all lines + meta
        let mut lines: Vec<Line<'static>> = content_lines
            .iter()
            .map(|line| Line::from(Span::styled(line.clone(), theme.tool_content_style())))
            .collect();
        if let Some(meta) = meta {
            lines.push(Line::from(Span::styled(meta, theme.tool_meta_style())));
        }
        Some(lines)
    } else if expanded {
        // Expanded: show tail view (summary + last N lines) + meta
        let above = total - MAX_CONTENT_LINES;
        let mut lines: Vec<Line<'static>> = vec![Line::from(Span::styled(
            format!("... ({above} lines above)"),
            theme.tool_truncated_style(),
        ))];
        let start = total - MAX_CONTENT_LINES;
        for line in &content_lines[start..] {
            lines.push(Line::from(Span::styled(line.clone(), theme.tool_content_style())));
        }
        if let Some(meta) = meta {
            lines.push(Line::from(Span::styled(meta, theme.tool_meta_style())));
        }
        Some(lines)
    } else {
        // Collapsed: caller will use FilledHeaderBar::collapsed(true)
        None
    }
}

/// Build content lines for a pending tool call.
fn build_pending_content_lines(output: &str, theme: &Theme) -> Vec<Line<'static>> {
    let output_lines: Vec<String> = output.lines().map(std::string::ToString::to_string).collect();
    if output_lines.is_empty() {
        return vec![Line::from(Span::styled(
            "⋯ executing".to_string(),
            theme.tool_meta_style(),
        ))];
    }

    let total = output_lines.len();
    let mut lines = Vec::new();

    if total > MAX_CONTENT_LINES {
        let above = total - MAX_CONTENT_LINES;
        lines.push(Line::from(Span::styled(
            format!("... ({above} lines above)"),
            theme.tool_truncated_style(),
        )));
        let start = total - MAX_CONTENT_LINES;
        for line in &output_lines[start..] {
            lines.push(Line::from(Span::styled(line.clone(), theme.tool_content_style())));
        }
    } else {
        for line in &output_lines {
            lines.push(Line::from(Span::styled(line.clone(), theme.tool_content_style())));
        }
    }

    lines.push(Line::from(Span::styled(
        "⋯ executing".to_string(),
        theme.tool_meta_style(),
    )));
    lines
}

impl HeightAware for ToolCallMessage {
    fn height(&self, width: usize, _theme: &dyn ratatui_md::MdTheme) -> usize {
        if width == 0 {
            return 4;
        }

        // Pending tool calls
        if let ToolCallVariant::Pending {
            output,
            ..
        } = &self.variant
        {
            let content_lines = build_pending_content_lines(output, &Theme::gh_dark());
            let widget = FilledHeaderBar::new("pending").content(content_lines);
            return widget.height(width);
        }

        // Non-pending variants
        let (content_lines, has_error, expanded, meta) = match &self.variant {
            ToolCallVariant::Read(msg) => (
                msg.content_lines(),
                msg.has_error,
                msg.is_expanded(),
                None,
            ),
            ToolCallVariant::Write(msg) => (
                msg.content_lines(),
                msg.has_error,
                msg.is_expanded(),
                Some(format!("({} bytes)", msg.content.len())),
            ),
            ToolCallVariant::Edit(msg) => (
                msg.content_lines(),
                msg.has_error,
                msg.is_expanded(),
                Some(format!("({} edit(s))", msg.edits.len())),
            ),
            ToolCallVariant::Bash(msg) => (msg.content_lines(), msg.has_error, msg.is_expanded(), None),
            ToolCallVariant::Unknown(msg) => (msg.content_lines(), msg.has_error, msg.is_expanded(), None),
            ToolCallVariant::Pending { .. } => unreachable!(),
        };

        let theme = Theme::gh_dark();
        let content = build_content_lines(&content_lines, has_error, expanded, meta, &theme);
        let collapsed = content.is_none();
        let content_lines_for_height = content.unwrap_or_default();

        let widget = FilledHeaderBar::new("tool")
            .content(content_lines_for_height)
            .collapsed(collapsed);
        widget.height(width)
    }
}

impl ToolCallMessage {
    pub fn render_into(&self, area: Rect, buf: &mut Buffer, theme: &Theme, rtk_active: bool) {
        if area.width == 0 || area.height == 0 {
            return;
        }

        // Pending variant
        if let ToolCallVariant::Pending {
            tool_name,
            tool_args,
            output,
            ..
        } = &self.variant
        {
            let raw_target = Self::extract_pending_target(tool_name, tool_args);
            let target = if tool_name == "bash" && rtk_active {
                if raw_target.starts_with("rtk") || raw_target.starts_with("rtk ") {
                    raw_target
                } else {
                    format!("rtk {raw_target}")
                }
            } else {
                raw_target
            };

            let header_text = format!("{tool_name}: {target}");
            let content_lines = build_pending_content_lines(output, theme);
            let widget = FilledHeaderBar::new(&header_text)
                .header_color(theme.tool_header_color(tool_name, false))
                .border_color(theme.tool_header_color(tool_name, false))
                .content(content_lines)
                .content_bg(theme.bg)
                .content_fg(theme.text)
                .collapsed(false);

            widget.render_into(area, buf);
            return;
        }

        // Non-pending variants
        let (tool_name, target, meta, has_error, _expanded) = match &self.variant {
            ToolCallVariant::Read(msg) => (
                "read",
                msg.path.clone(),
                None,
                msg.has_error,
                msg.is_expanded(),
            ),
            ToolCallVariant::Write(msg) => (
                "write",
                msg.path.clone(),
                Some(format!("({} bytes)", msg.content.len())),
                msg.has_error,
                msg.is_expanded(),
            ),
            ToolCallVariant::Edit(msg) => (
                "edit",
                msg.path.clone(),
                Some(format!("({} edit(s))", msg.edits.len())),
                msg.has_error,
                msg.is_expanded(),
            ),
            ToolCallVariant::Bash(msg) => {
                let (target, meta) = if rtk_active {
                    let display = if msg.command.starts_with("rtk") || msg.command.starts_with("rtk ") {
                        msg.command.clone()
                    } else {
                        format!("rtk {}", msg.command)
                    };
                    (display, None)
                } else if let Some(ref original) = msg.rtk_original {
                    (original.clone(), Some(format!("(→ {})", msg.command)))
                } else {
                    (msg.command.clone(), None)
                };
                ("bash", target, meta, msg.has_error, msg.is_expanded())
            }
            ToolCallVariant::Unknown(msg) => (
                "unknown",
                msg.tool_name.clone(),
                None,
                msg.has_error,
                msg.is_expanded(),
            ),
            ToolCallVariant::Pending { .. } => unreachable!(),
        };

        let (content_lines, expanded) = match &self.variant {
            ToolCallVariant::Read(msg) => (msg.content_lines(), msg.is_expanded()),
            ToolCallVariant::Write(msg) => (msg.content_lines(), msg.is_expanded()),
            ToolCallVariant::Edit(msg) => (msg.content_lines(), msg.is_expanded()),
            ToolCallVariant::Bash(msg) => (msg.content_lines(), msg.is_expanded()),
            ToolCallVariant::Unknown(msg) => (msg.content_lines(), msg.is_expanded()),
            ToolCallVariant::Pending { .. } => unreachable!(),
        };

        let header_text = format!("{tool_name}: {target}");
        let header_color = theme.tool_header_color(tool_name, has_error);

        let content = build_content_lines(&content_lines, has_error, expanded, meta, theme);
        let collapsed = content.is_none();

        let widget = FilledHeaderBar::new(&header_text)
            .header_color(header_color)
            .border_color(header_color)
            .content(content.unwrap_or_default())
            .collapsed(collapsed)
            .content_bg(theme.bg)
            .content_fg(theme.text);

        widget.render_into(area, buf);
    }

    /// Extract a human-readable target from pending tool args.
    ///
    /// For `bash` → command, for `read`/`write`/`edit` → path.
    fn extract_pending_target(tool_name: &str, tool_args: &serde_json::Value) -> String {
        match tool_name {
            "bash" => tool_args.get("command").and_then(|v| v.as_str()).unwrap_or("unknown").to_string(),
            "read" | "write" | "edit" => {
                tool_args.get("path").and_then(|v| v.as_str()).unwrap_or("unknown").to_string()
            }
            _ => "unknown".to_string(),
        }
    }
}

// Trait for tool call variants to provide content info
pub trait ToolCallRender {
    fn content_lines(&self) -> Vec<String>;
    fn is_expanded(&self) -> bool;
}

/// Build a `ToolCallVariant` from tool name, result, and args.
/// This is a standalone function that can be used without an `App`.
#[must_use]
pub fn build_tool_call_variant(
    tool_name: &str,
    result: &str,
    has_error: bool,
    tool_args: Option<serde_json::Value>,
    rtk_original: Option<String>,
    expanded: bool,
) -> ToolCallVariant {
    match tool_name {
        "read" => ToolCallVariant::Read(ReadToolCall::from_args(result, has_error, tool_args.as_ref(), expanded)),
        "write" => ToolCallVariant::Write(WriteToolCall::from_args(result, has_error, tool_args.as_ref(), expanded)),
        "edit" => ToolCallVariant::Edit(EditToolCall::from_args(result, has_error, tool_args.as_ref(), expanded)),
        "bash" => ToolCallVariant::Bash(BashToolCall::from_args(result, has_error, tool_args.as_ref(), rtk_original, expanded)),
        _ => ToolCallVariant::Unknown(UnknownToolCall {
            tool_name: tool_name.to_string(),
            result: result.to_string(),
            has_error,
            expanded,
        }),
    }
}
