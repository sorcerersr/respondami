//! User message rendering.
//!
//! Renders user input messages with accent-colored text, top/bottom spacing,
//! and line wrapping. Implements `HeightAware` for accurate height computation.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Block, Paragraph, Widget, Wrap};

use ratatui_md::HeightAware;
use crate::tui::theme::Theme;

/// A user input message.
#[derive(Debug, Clone)]
pub struct UserMessage {
    pub content: String,
}

impl HeightAware for UserMessage {
    fn height(&self, width: usize, _theme: &dyn ratatui_md::MdTheme) -> usize {
        if width == 0 {
            return 4;
        }
        // 2 blank lines + content lines + 1 blank line
        let mut total_lines = 0;
        for line in self.content.lines() {
            total_lines += line.chars().count().div_ceil(width.max(1)).max(1);
        }
        total_lines + 3 // +2 for top spacing, +1 for bottom blank line
    }
}

impl UserMessage {
    pub fn render_into(&self, area: Rect, buf: &mut Buffer, theme: &Theme) {
        if area.width == 0 || area.height == 0 {
            return;
        }

        let mut lines: Vec<Line<'static>> = Vec::new();
        // 2 blank lines for top spacing
        lines.push(Line::default());
        lines.push(Line::default());
        // Content in accent blue, no indent
        for line in self.content.lines() {
            lines.push(Line::from(vec![
                Span::styled(line.to_string(), theme.accent_bold_style()),
            ]));
        }
        // 1 blank line for bottom spacing
        lines.push(Line::default());

        // Fill area with bg first, then render text on top.
        let bg = Block::new().style(Style::default().bg(theme.bg));
        bg.render(area, buf);

        let para = Paragraph::new(Text::from(lines))
            .style(Style::default().bg(theme.bg))
            .wrap(Wrap { trim: true });
        para.render(area, buf);
    }
}
