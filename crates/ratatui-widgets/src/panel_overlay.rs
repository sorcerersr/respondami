//! Panel overlay widget.
//!
//! A generic overlay panel with a title bar, borders, and content lines.
//! Uses a custom border set with asymmetric corners.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::symbols::border;
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Block, Borders, Paragraph, Widget, Wrap};

/// Lightweight border set for panel overlays.
/// Thin verticals and light horizontals reduce visual weight for transient popups.
/// Asymmetric: angled top corners (▟/▜) for header feel, flat bottom (▔/▔) to ground.
pub const PANEL_BORDER: border::Set = border::Set {
    top_left: "▟",
    top_right: "▜",
    bottom_left: "▔",
    bottom_right: "▔",
    vertical_left: "▏",
    vertical_right: "▕",
    horizontal_top: "▔",
    horizontal_bottom: "▔",
};

/// A generic overlay panel with title bar, borders, and content lines.
pub struct PanelOverlay<'a> {
    /// Title text displayed in the title bar.
    title: &'a str,
    /// Border style (fg color).
    border_style: Style,
    /// Title bar style (fg + bg, typically reversed).
    title_style: Style,
    /// Content lines.
    content: Vec<Line<'a>>,
    /// Background color of the content area.
    content_bg: ratatui::style::Color,
    /// Index of the selected item (for highlighting).
    selected: Option<usize>,
    /// Style for the selected item.
    selected_style: Style,
    /// Style for non-selected items.
    content_style: Style,
}

impl<'a> PanelOverlay<'a> {
    /// Create a new `PanelOverlay` with the given title.
    pub fn new(title: &'a str) -> Self {
        Self {
            title,
            border_style: Style::default().fg(ratatui::style::Color::Blue),
            title_style: Style::default()
                .fg(ratatui::style::Color::Blue)
                .bg(ratatui::style::Color::Rgb(0x2a, 0x31, 0x3c))
                .add_modifier(ratatui::style::Modifier::BOLD)
                .add_modifier(ratatui::style::Modifier::REVERSED),
            content: Vec::new(),
            content_bg: ratatui::style::Color::Rgb(0x2a, 0x31, 0x3c),
            selected: None,
            selected_style: Style::default()
                .fg(ratatui::style::Color::Rgb(0xd1, 0xd7, 0xe0))
                .bg(ratatui::style::Color::Rgb(0x47, 0x8b, 0xe6))
                .add_modifier(ratatui::style::Modifier::BOLD),
            content_style: Style::default()
                .fg(ratatui::style::Color::Rgb(0xd1, 0xd7, 0xe0))
                .bg(ratatui::style::Color::Rgb(0x2a, 0x31, 0x3c)),
        }
    }

    /// Set the border style.
    pub fn border_style(mut self, style: Style) -> Self {
        self.border_style = style;
        self
    }

    /// Set the title bar style.
    pub fn title_style(mut self, style: Style) -> Self {
        self.title_style = style;
        self
    }

    /// Set the content lines.
    pub fn content(mut self, content: Vec<Line<'a>>) -> Self {
        self.content = content;
        self
    }

    /// Set the content background color.
    pub fn content_bg(mut self, color: ratatui::style::Color) -> Self {
        self.content_bg = color;
        self
    }

    /// Set the index of the selected item.
    pub fn selected(mut self, index: usize) -> Self {
        self.selected = Some(index);
        self
    }

    /// Set the selected item style.
    pub fn selected_style(mut self, style: Style) -> Self {
        self.selected_style = style;
        self
    }

    /// Set the non-selected content style.
    pub fn content_style(mut self, style: Style) -> Self {
        self.content_style = style;
        self
    }

    /// Calculate the height of the widget.
    pub fn height(&self) -> usize {
        let content_height = self.content.len().max(1);
        content_height + 2 // top border + bottom border
    }

    /// Render the widget into the buffer.
    pub fn render_into(&self, area: Rect, buf: &mut Buffer) {
        if area.width < 4 || area.height < 2 {
            return;
        }

        // Build styled content lines, preserving per-span foreground colors
        let styled_content: Vec<Line> = self
            .content
            .iter()
            .enumerate()
            .map(|(i, line)| {
                let base_style = if self.selected == Some(i) {
                    self.selected_style
                } else {
                    self.content_style
                };
                Line::from(
                    line.spans.iter().map(|span| {
                        // Preserve span's fg, inject bg from content/selected style
                        Span::styled(span.content.clone(), base_style.patch(span.style))
                    }).collect::<Vec<_>>(),
                )
            })
            .collect();

        // Fill remaining rows with empty styled lines
        let total_needed = (area.height - 2).max(1) as usize;
        let mut all_lines = styled_content;
        while all_lines.len() < total_needed {
            all_lines.push(Line::from(Span::styled(
                "".to_string(),
                Style::default().bg(self.content_bg),
            )));
        }

        let block = Block::default()
            .borders(Borders::ALL)
            .border_set(PANEL_BORDER)
            .border_style(self.border_style)
            .title(Span::styled(self.title, self.title_style));

        let text = Text::from(all_lines);
        let paragraph = Paragraph::new(text)
            .block(block)
            .wrap(Wrap { trim: true })
            .style(Style::default().bg(self.content_bg));

        paragraph.render(area, buf);
    }
}

impl<'a> Widget for PanelOverlay<'a> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        self.render_into(area, buf);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn height_minimum() {
        let overlay = PanelOverlay::new(" Test ");
        // With no content, height is max(1, 0) + 2 = 3
        assert_eq!(overlay.height(), 3);
    }

    #[test]
    fn height_with_content() {
        let overlay = PanelOverlay::new(" Test ")
            .content(vec![Line::from("line 1"), Line::from("line 2")]);
        assert_eq!(overlay.height(), 4); // 2 content + 2 borders
    }

    #[test]
    fn render_basic() {
        let overlay = PanelOverlay::new(" Title ")
            .content(vec![Line::from("Content")]);
        let mut buf = Buffer::empty(Rect::new(0, 0, 10, 3));
        overlay.render_into(Rect::new(0, 0, 10, 3), &mut buf);
        // Title should be present in first row
        let has_title = buf.content.iter().take(10).any(|cell| cell.symbol() == "T");
        assert!(has_title);
    }

    #[test]
    fn render_selected() {
        let overlay = PanelOverlay::new(" List ")
            .content(vec![Line::from("item 1"), Line::from("item 2")])
            .selected(1);
        let mut buf = Buffer::empty(Rect::new(0, 0, 10, 4));
        overlay.render_into(Rect::new(0, 0, 10, 4), &mut buf);
        // Second content line should have accent bg
        assert_eq!(buf[(1, 2)].bg, ratatui::style::Color::Rgb(0x47, 0x8b, 0xe6));
    }
}
