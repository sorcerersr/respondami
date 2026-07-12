//! Assistant message rendering.
//!
//! Renders assistant responses as markdown content blocks using `ratatui_md`.
//! Implements `HeightAware` for accurate height computation in the chat viewport.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;

use ratatui_md::{render_block, HeightAware, MarkdownRenderer};
use crate::tui::theme::Theme;

/// An assistant response message. Rendered as markdown content blocks.
#[derive(Debug, Clone)]
pub struct AssistantMessage {
    pub content: String,
}

impl HeightAware for AssistantMessage {
    fn height(&self, width: usize, theme: &dyn ratatui_md::MdTheme) -> usize {
        if width == 0 || self.content.is_empty() {
            return 1;
        }
        let blocks = MarkdownRenderer::new(theme).render(&self.content);
        let total: usize = blocks.iter().map(|b| b.height(width, theme)).sum();
        total.max(1)
    }
}

impl AssistantMessage {
    /// Render this message into the given buffer area.
    /// Renders all blocks in full (no clipping — `ScrollView` handles viewport).
    pub fn render_into(&self, area: Rect, buf: &mut Buffer, theme: &Theme) {
        if area.width == 0 || area.height == 0 || self.content.is_empty() {
            return;
        }

        let blocks = MarkdownRenderer::new(theme).render(&self.content);
        let mut y_offset = 0u16;

        for block in &blocks {
            if y_offset >= area.height {
                break;
            }

            let block_height = block.height(area.width as usize, theme) as u16;
            let remaining = area.height - y_offset;
            let render_height = block_height.min(remaining);

            if render_height > 0 {
                let block_area = Rect {
                    x: area.x,
                    y: area.y + y_offset,
                    width: area.width,
                    height: render_height,
                };
                render_block(block, block_area, buf, theme);
            }

            y_offset += render_height;
        }
    }
}
