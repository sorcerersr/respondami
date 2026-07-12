//! Thinking/reasoning message rendering.
//!
//! Renders thinking blocks with three display modes: Hidden ("Thinking..." only),
//! Collapsed (with token count), and Expanded (header + recent thinking lines).

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Block, Paragraph, Widget, Wrap};

use crate::tui::theme::Theme;
use crate::tui::thinking_display::ThinkingDisplay;

/// A thinking/reasoning block message.
#[derive(Debug, Clone)]
pub struct ThinkingMessage {
    pub reasoning: String,
}

impl ThinkingMessage {
    /// Compute height given display mode and max lines.
    #[must_use]
    pub fn height_with(
        &self,
        width: usize,
        display: ThinkingDisplay,
        max_lines: usize,
        _theme: &Theme,
    ) -> usize {
        if width == 0 {
            return 2;
        }
        match display {
            ThinkingDisplay::Hidden | ThinkingDisplay::Collapsed => 2,
            ThinkingDisplay::Expanded => {
                if self.reasoning.is_empty() {
                    return 2;
                }
                let total_lines = self.reasoning.lines().count();
                let visible = total_lines.min(max_lines);
                4 + visible
            }
        }
    }

    pub fn render_into(
        &self,
        area: Rect,
        display: ThinkingDisplay,
        max_lines: usize,
        buf: &mut Buffer,
        theme: &Theme,
    ) {
        if area.width == 0 || area.height == 0 {
            return;
        }

        match display {
            ThinkingDisplay::Hidden => {
                self.render_hidden(area, buf, theme);
            }
            ThinkingDisplay::Collapsed => {
                self.render_collapsed(area, buf, theme);
            }
            ThinkingDisplay::Expanded => {
                self.render_expanded(area, max_lines, buf, theme);
            }
        }
    }

    fn render_hidden(&self, area: Rect, buf: &mut Buffer, theme: &Theme) {
        let bg = Block::new().style(Style::default().bg(theme.bg));
        bg.render(area, buf);

        let line = Line::from(Span::styled(
            "Thinking...",
            Style::default()
                .fg(theme.text_muted)
                .bg(theme.bg)
                .add_modifier(Modifier::ITALIC),
        ));
        let para = Paragraph::new(Text::from(vec![line, Line::default()]))
            .style(Style::default().bg(theme.bg));
        para.render(area, buf);
    }

    fn render_collapsed(&self, area: Rect, buf: &mut Buffer, theme: &Theme) {
        let bg = Block::new().style(Style::default().bg(theme.bg));
        bg.render(area, buf);

        let icon = ThinkingDisplay::Collapsed.icon();
        let token_count = self.reasoning.split_whitespace().count();
        let text = if icon.is_empty() {
            format!("Thinking... ({token_count} tokens)")
        } else {
            format!("Thinking... {icon} ({token_count} tokens)")
        };
        let line = Line::from(Span::styled(
            text,
            Style::default()
                .fg(theme.text_muted)
                .bg(theme.bg)
                .add_modifier(Modifier::ITALIC),
        ));
        let para = Paragraph::new(Text::from(vec![line, Line::default()]))
            .style(Style::default().bg(theme.bg));
        para.render(area, buf);
    }

    fn render_expanded(
        &self,
        area: Rect,
        max_lines: usize,
        buf: &mut Buffer,
        theme: &Theme,
    ) {
        if self.reasoning.is_empty() {
            self.render_collapsed(area, buf, theme);
            return;
        }

        let total_lines = self.reasoning.lines().count();
        let visible_lines = total_lines.min(max_lines);
        let token_count = self.reasoning.split_whitespace().count();
        let icon = ThinkingDisplay::Expanded.icon();

        let mut lines: Vec<Line<'static>> = Vec::new();

        // Header
        let is_truncated = total_lines > max_lines;
        let header_text = if is_truncated {
            format!(
                "▸ Thinking ({token_count} tokens) {icon} (showing last {max_lines} of {total_lines} lines)"
            )
        } else {
            format!("▸ Thinking ({token_count} tokens) {icon}")
        };
        lines.push(Line::from(Span::styled(
            header_text,
            theme.thinking_header_style(),
        )));

        // Separator
        lines.push(Line::from(Span::styled(
            "──────────────────────────────────────────────────",
            Style::default().fg(theme.border).bg(theme.bg),
        )));

        // Body — last `visible_lines` lines, indented with 2 spaces
        let body_lines: Vec<&str> = self.reasoning.lines().collect();
        let start = total_lines.saturating_sub(visible_lines);
        for line in &body_lines[start..] {
            lines.push(Line::from(Span::styled(
                format!("  {line}"),
                theme.thinking_body_style(),
            )));
        }

        // Separator + blank
        lines.push(Line::from(Span::styled(
            "──────────────────────────────────────────────────",
            Style::default().fg(theme.border).bg(theme.bg),
        )));
        lines.push(Line::from(Span::styled(
            "",
            Style::default().bg(theme.bg),
        )));

        // Fill area with bg first, then render text on top.
        let bg = Block::new().style(Style::default().bg(theme.bg));
        bg.render(area, buf);

        let para = Paragraph::new(Text::from(lines))
            .style(Style::default().bg(theme.bg))
            .wrap(Wrap { trim: true });
        para.render(area, buf);
    }
}
