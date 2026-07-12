//! System message rendering.
//!
//! Renders system messages (info, errors, status) as dimmed text with line wrapping.
//! Implements `HeightAware` for accurate height computation.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Block, Paragraph, Widget, Wrap};

use ratatui_md::HeightAware;
use crate::tui::theme::Theme;

/// A system message (info, errors, status).
#[derive(Debug, Clone)]
pub struct SystemMessage {
    pub content: String,
}

impl HeightAware for SystemMessage {
    fn height(&self, width: usize, _theme: &dyn ratatui_md::MdTheme) -> usize {
        if width == 0 {
            return 2;
        }
        // Count rows needed: each line() is one row, long lines wrap.
        let rows: usize = self.content.lines().map(|line| {
            line.chars().count().div_ceil(width).max(1)
        }).sum();
        rows.max(1) + 1
    }
}

impl SystemMessage {
    pub fn render_into(&self, area: Rect, buf: &mut Buffer, theme: &Theme) {
        if area.width == 0 || area.height == 0 {
            return;
        }

        // Fill area with bg first, then render text on top.
        // Block reliably fills all cells; Paragraph may leave empty rows unfilled.
        let bg = Block::new().style(Style::default().bg(theme.bg));
        bg.render(area, buf);

        let lines: Vec<Line<'static>> = self.content.lines().map(|line| {
            Line::from(Span::styled(line.to_string(), theme.text_dim_style()))
        }).collect();
        let text = Text::from(lines);
        let para = Paragraph::new(text)
            .style(Style::default().bg(theme.bg))
            .wrap(Wrap { trim: true });
        para.render(area, buf);
    }
}
