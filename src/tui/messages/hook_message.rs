//! Hook message rendering.
//!
//! Hook messages display in three modes:
//! - **Hidden**: Messages are not rendered at all (dropped in event processing).
//! - **Minimal**: A single dimmed line with status icon and hook origin.
//! - **Full**: A purple-bordered expanded box with output.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::prelude::Widget;
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

use super::super::hook_display::HookDisplay;
use super::super::theme::Theme;
use ratatui_widgets::FilledHeaderBar;
use crate::hooks::HookEvent;

/// A hook message for display in the chat.
#[derive(Debug, Clone)]
pub struct HookMessage {
    /// The hook event type.
    pub event: HookEvent,
    /// The hook name (script filename).
    pub hook_name: String,
    /// Whether the hook was successful (exit 0).
    pub success: bool,
    /// Standard output from the hook.
    pub stdout: String,
    /// Standard error from the hook (if any).
    pub stderr: String,
    /// Associated tool name (for PreToolUse/PostToolUse).
    pub tool_name: Option<String>,
}

impl HookMessage {
    /// Compute the height of the message in lines.
    #[must_use]
    pub fn height(&self, width: usize, display: HookDisplay, _theme: &Theme) -> usize {
        match display {
            HookDisplay::Hidden => 0, // never rendered
            HookDisplay::Minimal => 1, // single line
            HookDisplay::Full => {
                // Full mode: always expanded purple box
                let indicator = if self.success { "✓" } else { "✗" };
                let event_str = format!("{}", self.event);
                let tool_suffix = self
                    .tool_name
                    .as_ref()
                    .map(|t| format!(" ({t})"))
                    .unwrap_or_default();
                let header_text = format!(
                    "{} hook: {} - {}{}",
                    indicator, event_str, self.hook_name, tool_suffix
                );

                let content_lines = self.build_content_lines();
                let widget = FilledHeaderBar::new(&header_text)
                    .header_color(Color::Rgb(100, 80, 140))
                    .border_color(Color::Rgb(100, 80, 140))
                    .content(content_lines)
                    .collapsed(false) // always expanded
                    .content_bg(Color::Rgb(30, 30, 46))
                    .content_fg(Color::White);
                widget.height(width)
            }
        }
    }

    /// Build content lines for the expanded Full mode.
    fn build_content_lines(&self) -> Vec<Line<'_>> {
        let mut lines = Vec::new();
        if !self.stdout.is_empty() {
            for line in self.stdout.lines() {
                lines.push(Line::from(Span::raw(line.to_string())));
            }
        }
        if !self.stderr.is_empty() {
            for line in self.stderr.lines() {
                lines.push(Line::from(Span::styled(
                    line.to_string(),
                    Style::default().fg(Color::Red),
                )));
            }
        }
        if lines.is_empty() {
            lines.push(Line::from(Span::styled(
                "[no output]".to_string(),
                Style::default().fg(Color::Gray),
            )));
        }
        lines
    }

    /// Render the hook message into the buffer.
    pub fn render_into(&self, area: Rect, display: HookDisplay, buf: &mut Buffer, theme: &Theme) {
        let width = area.width as usize;
        if width == 0 || area.height == 0 {
            return;
        }

        match display {
            HookDisplay::Hidden => {
                // Never rendered — event is dropped in processing
            }
            HookDisplay::Minimal => {
                self.render_minimal(area, buf, theme);
            }
            HookDisplay::Full => {
                self.render_full(area, buf);
            }
        }
    }

    /// Render in minimal mode: single dimmed line with muted semantic colors.
    fn render_minimal(&self, area: Rect, buf: &mut Buffer, theme: &Theme) {
        let indicator = if self.success { "✓" } else { "✗" };
        let indicator_color = if self.success {
            Color::Rgb(74, 122, 74) // dark green
        } else {
            Color::Rgb(122, 74, 74) // dark red
        };
        let dimmed = Color::Rgb(130, 130, 130);

        let event_str = format!("{}", self.event);
        let line = Line::from(vec![
            Span::styled(indicator.to_string(), Style::default().fg(indicator_color)),
            Span::styled(
                format!(" hook: {} - {}", event_str, self.hook_name),
                Style::default().fg(dimmed),
            ),
        ]);

        let paragraph = Paragraph::new(line).style(theme.bg_style());
        paragraph.render(area, buf);
    }

    /// Render in full mode: expanded purple box with output.
    fn render_full(&self, area: Rect, buf: &mut Buffer) {
        let indicator = if self.success { "✓" } else { "✗" };
        let event_str = format!("{}", self.event);
        let tool_suffix = self
            .tool_name
            .as_ref()
            .map(|t| format!(" ({t})"))
            .unwrap_or_default();
        let header_text = format!(
            "{} hook: {} - {}{}",
            indicator, event_str, self.hook_name, tool_suffix
        );

        let content_lines = self.build_content_lines();

        let widget = FilledHeaderBar::new(&header_text)
            .header_color(Color::Rgb(100, 80, 140))
            .border_color(Color::Rgb(100, 80, 140))
            .content(content_lines)
            .collapsed(false) // always expanded
            .content_bg(Color::Rgb(30, 30, 46))
            .content_fg(Color::White);

        widget.render_into(area, buf);
    }
}
