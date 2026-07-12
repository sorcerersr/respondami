//! Command palette overlay widget.
//!
//! A search-based command palette with an input row, divider, and command list.
//! Grows upward from a given anchor point.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Block, Borders, Paragraph, Widget, Wrap};

use super::panel_overlay::PANEL_BORDER;

/// A command palette with search input and command list.
pub struct CommandPaletteOverlay {
    /// Search query text.
    query: String,
    /// Commands as (name, description) pairs.
    commands: Vec<(String, String)>,
    /// Index of the selected command.
    selected: usize,
    /// Prefix for the input row (e.g., "> " or "/").
    prefix: String,
    /// Border style.
    border_style: Style,
    /// Title style.
    title_style: Style,
    /// Content background color.
    content_bg: ratatui::style::Color,
    /// Text color.
    text_color: ratatui::style::Color,
    /// Accent color (for divider, cursor, selected bg).
    accent_color: ratatui::style::Color,
    /// Scroll offset for virtual scrolling.
    scroll_offset: usize,
    /// Title text.
    title: String,
    /// Maximum visible commands.
    max_visible: usize,
}

impl Default for CommandPaletteOverlay {
    fn default() -> Self {
        Self::new()
    }
}

impl CommandPaletteOverlay {
    /// Create a new `CommandPaletteOverlay`.
    pub fn new() -> Self {
        Self {
            query: String::new(),
            commands: Vec::new(),
            selected: 0,
            prefix: "> ".to_string(),
            border_style: Style::default().fg(ratatui::style::Color::Rgb(0x6c, 0xb6, 0xff)),
            title_style: Style::default()
                .fg(ratatui::style::Color::Rgb(0x6c, 0xb6, 0xff))
                .bg(ratatui::style::Color::Rgb(0x2a, 0x31, 0x3c))
                .add_modifier(ratatui::style::Modifier::BOLD)
                .add_modifier(ratatui::style::Modifier::REVERSED),
            content_bg: ratatui::style::Color::Rgb(0x2a, 0x31, 0x3c),
            text_color: ratatui::style::Color::Rgb(0xd1, 0xd7, 0xe0),
            accent_color: ratatui::style::Color::Rgb(0x47, 0x8b, 0xe6),
            title: " Commands ".to_string(),
            max_visible: 10,
            scroll_offset: 0,
        }
    }

    /// Set the search query.
    pub fn query(mut self, query: &str) -> Self {
        self.query = query.to_string();
        self
    }

    /// Set the commands.
    pub fn commands(mut self, commands: Vec<(String, String)>) -> Self {
        self.commands = commands;
        self
    }

    /// Set the selected command index.
    pub fn selected(mut self, selected: usize) -> Self {
        self.selected = selected;
        self
    }

    /// Set the input row prefix.
    pub fn prefix(mut self, prefix: &str) -> Self {
        self.prefix = prefix.to_string();
        self
    }

    /// Set the border style.
    pub fn border_style(mut self, style: Style) -> Self {
        self.border_style = style;
        self
    }

    /// Set the title style.
    pub fn title_style(mut self, style: Style) -> Self {
        self.title_style = style;
        self
    }

    /// Set the content background color.
    pub fn content_bg(mut self, color: ratatui::style::Color) -> Self {
        self.content_bg = color;
        self
    }

    /// Set the text color.
    pub fn text_color(mut self, color: ratatui::style::Color) -> Self {
        self.text_color = color;
        self
    }

    /// Set the accent color.
    pub fn accent_color(mut self, color: ratatui::style::Color) -> Self {
        self.accent_color = color;
        self
    }

    /// Set the title text.
    pub fn title(mut self, title: &str) -> Self {
        self.title = title.to_string();
        self
    }

    /// Set the maximum visible commands.
    pub fn max_visible(mut self, max: usize) -> Self {
        self.max_visible = max;
        self
    }

    /// Set the virtual scroll offset.
    pub fn scroll_offset(mut self, offset: usize) -> Self {
        self.scroll_offset = offset;
        self
    }

    /// Calculate the height of the widget.
    pub fn height(&self) -> usize {
        let visible = self.commands.len().min(self.max_visible);
        // 1 (top border) + 1 (input) + N (commands) + 1 (bottom border)
        3 + visible
    }

    /// Render the widget into the buffer.
    pub fn render_into(&self, area: Rect, buf: &mut Buffer) {
        if area.width < 10 || area.height < 4 {
            return;
        }

        let mut lines: Vec<Line> = Vec::new();

        // Input row: prefix + query + cursor bar
        let input_text = format!("{}{}", self.prefix, self.query);
        let cursor_col = input_text.len();
        let before = &input_text[..cursor_col.min(input_text.len())];
        let after = &input_text[cursor_col.min(input_text.len())..];

        let mut input_spans = Vec::new();
        if !before.is_empty() {
            input_spans.push(Span::styled(
                before.to_string(),
                Style::default().fg(self.text_color).bg(self.content_bg),
            ));
        }
        input_spans.push(Span::styled(
            "|".to_string(),
            Style::default().fg(self.accent_color).add_modifier(ratatui::style::Modifier::BOLD),
        ));
        if !after.is_empty() {
            input_spans.push(Span::styled(
                after.to_string(),
                Style::default().fg(self.text_color).bg(self.content_bg),
            ));
        }
        lines.push(Line::from(input_spans));

        // Command list rows with virtual scrolling
        let page_size = (area.height - 3).max(1) as usize; // -3: top border + input row + bottom border
        let visible_count = self.commands.len().min(self.max_visible);
        let mut offset = self.scroll_offset;
        if self.selected >= offset + page_size {
            offset = self.selected - page_size + 1;
        } else if self.selected < offset {
            offset = self.selected;
        }
        offset = offset.min(visible_count.saturating_sub(page_size));
        let visible_end = (offset + page_size).min(visible_count);

        let name_width = 20u16;
        for (i, (name, desc)) in self.commands[offset..visible_end].iter().enumerate() {
            let is_selected = (offset + i) == self.selected;
            let padded_name = format!(
                "{: <name_width$}",
                name,
                name_width = name_width as usize
            );
            let display = format!("{}{}", padded_name, desc);

            let style = if is_selected {
                Style::default()
                    .fg(self.text_color)
                    .bg(self.accent_color)
                    .add_modifier(ratatui::style::Modifier::BOLD)
            } else {
                Style::default().fg(self.text_color).bg(self.content_bg)
            };
            lines.push(Line::from(Span::styled(display, style)));
        }

        // Fill remaining rows
        let total_needed = (area.height - 3).max(1) as usize; // -3: top border + input + bottom border
        while lines.len() < total_needed {
            lines.push(Line::from(Span::styled(
                "".to_string(),
                Style::default().bg(self.content_bg),
            )));
        }

        let block = Block::default()
            .borders(Borders::ALL)
            .border_set(PANEL_BORDER)
            .border_style(self.border_style)
            .title(Span::styled(&self.title, self.title_style));

        let text = Text::from(lines);
        let paragraph = Paragraph::new(text)
            .block(block)
            .wrap(Wrap { trim: true })
            .style(Style::default().bg(self.content_bg));

        paragraph.render(area, buf);
    }
}

impl Widget for CommandPaletteOverlay {
    fn render(self, area: Rect, buf: &mut Buffer) {
        self.render_into(area, buf);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn height_minimum() {
        let palette = CommandPaletteOverlay::new();
        assert_eq!(palette.height(), 3); // 3 + 0 commands
    }

    #[test]
    fn height_with_commands() {
        let palette = CommandPaletteOverlay::new()
            .commands(vec![("cmd1".into(), "desc1".into()), ("cmd2".into(), "desc2".into())]);
        assert_eq!(palette.height(), 5); // 3 + 2 commands
    }

    #[test]
    fn render_basic() {
        let palette = CommandPaletteOverlay::new()
            .query("test")
            .commands(vec![("Toggle".into(), "[collapsed → expanded]".into())])
            .selected(0);
        let mut buf = Buffer::empty(Rect::new(0, 0, 40, 5));
        palette.render_into(Rect::new(0, 0, 40, 5), &mut buf);
        // Input row should have prefix "> " followed by "test"
        assert_eq!(buf[(1, 1)].symbol(), ">");
        assert_eq!(buf[(3, 1)].symbol(), "t");
    }
}
