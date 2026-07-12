//! Compaction message rendering.
//!
//! Renders compaction results as a dimmed line showing messages compacted and
//! tokens saved. Implements `HeightAware` for height computation.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Block, Paragraph, Widget, Wrap};

use ratatui_md::HeightAware;
use crate::tui::theme::Theme;

/// A compaction message showing context summarization results.
#[derive(Debug, Clone)]
pub struct CompactionMessage {
    pub tokens_saved: u32,
    pub message_count: u32,
}

impl HeightAware for CompactionMessage {
    fn height(&self, width: usize, _theme: &dyn ratatui_md::MdTheme) -> usize {
        if width == 0 {
            return 2;
        }
        let text = format!("▸ Compacted {} messages (saved {} tokens)", self.message_count, self.tokens_saved);
        text.chars().count().div_ceil(width).max(1) + 1
    }
}

impl CompactionMessage {
    pub fn render_into(&self, area: Rect, buf: &mut Buffer, theme: &Theme) {
        if area.width == 0 || area.height == 0 {
            return;
        }

        // Fill area with bg first, then render text on top.
        let bg = Block::new().style(Style::default().bg(theme.bg));
        bg.render(area, buf);

        let text = format!("▸ Compacted {} messages (saved {} tokens)", self.message_count, self.tokens_saved);
        let line = Line::from(Span::styled(text, theme.text_muted_style()));
        let para = Paragraph::new(Text::from(vec![line, Line::default()]))
            .style(Style::default().bg(theme.bg))
            .wrap(Wrap { trim: true });
        para.render(area, buf);
    }
}
