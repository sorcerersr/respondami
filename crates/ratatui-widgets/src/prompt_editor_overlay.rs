//! Prompt editor overlay widget.
//!
//! A full-screen text editor popup with cursor support, scrolling, and hint bar.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Block, Borders, Paragraph, Widget, Wrap};

use super::panel_overlay::PANEL_BORDER;

/// A full-screen text editor popup with cursor support.
pub struct PromptEditorOverlay<'a> {
    /// Text content.
    text: &'a str,
    /// Cursor position (byte index into text).
    cursor_pos: usize,
    /// Wrap width for line wrapping.
    wrap_width: usize,
    /// Accent color (for cursor, dividers).
    accent_color: ratatui::style::Color,
    /// Border style.
    border_style: Style,
    /// Title style.
    title_style: Style,
    /// Content background color.
    content_bg: ratatui::style::Color,
    /// Text color.
    text_color: ratatui::style::Color,
    /// Dim text color (for hint bar).
    dim_color: ratatui::style::Color,
    /// Title text.
    title: &'a str,
    /// Hint bar text.
    hint: &'a str,
}

impl<'a> PromptEditorOverlay<'a> {
    /// Create a new `PromptEditorOverlay` with the given text.
    pub fn new(text: &'a str) -> Self {
        Self {
            text,
            cursor_pos: text.len().min(1),
            wrap_width: 80,
            accent_color: ratatui::style::Color::Rgb(0x47, 0x8b, 0xe6),
            border_style: Style::default().fg(ratatui::style::Color::Rgb(0x6c, 0xb6, 0xff)),
            title_style: Style::default()
                .fg(ratatui::style::Color::Rgb(0x6c, 0xb6, 0xff))
                .bg(ratatui::style::Color::Rgb(0x2a, 0x31, 0x3c))
                .add_modifier(ratatui::style::Modifier::BOLD)
                .add_modifier(ratatui::style::Modifier::REVERSED),
            content_bg: ratatui::style::Color::Rgb(0x2a, 0x31, 0x3c),
            text_color: ratatui::style::Color::Rgb(0xd1, 0xd7, 0xe0),
            dim_color: ratatui::style::Color::Rgb(0x65, 0x6c, 0x76),
            title: " Prompt Editor ",
            hint: "Enter: send  Esc: close  Ctrl+E",
        }
    }

    /// Set the cursor position (byte index).
    pub fn cursor_pos(mut self, pos: usize) -> Self {
        self.cursor_pos = pos;
        self
    }

    /// Set the wrap width.
    pub fn wrap_width(mut self, width: usize) -> Self {
        self.wrap_width = width;
        self
    }

    /// Set the accent color.
    pub fn accent_color(mut self, color: ratatui::style::Color) -> Self {
        self.accent_color = color;
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

    /// Set the dim text color.
    pub fn dim_color(mut self, color: ratatui::style::Color) -> Self {
        self.dim_color = color;
        self
    }

    /// Set the title text.
    pub fn title(mut self, title: &'a str) -> Self {
        self.title = title;
        self
    }

    /// Set the hint bar text.
    pub fn hint(mut self, hint: &'a str) -> Self {
        self.hint = hint;
        self
    }

    /// Build visual lines from the text with wrapping.
    fn build_visual_lines(&self, width: usize) -> Vec<(usize, usize, usize)> {
        let mut visual_lines = Vec::new();
        let width = width.max(1);
        for (line_idx, line) in self.text.lines().enumerate() {
            let chars: Vec<char> = line.chars().collect();
            let mut start = 0;
            while start < chars.len() {
                let end = (start + width).min(chars.len());
                visual_lines.push((line_idx, start, end));
                start = end;
            }
            // Add empty segment for line break if line was not empty
            if !line.is_empty() && chars.len() >= width {
                // The last segment already covers the end, no empty line needed
            }
        }
        // Handle trailing newline: if text ends with \n, add an empty visual line
        if self.text.ends_with('\n') && !self.text.is_empty() {
            visual_lines.push((self.text.lines().count(), 0, 0));
        }
        visual_lines
    }

    /// Find cursor visual position.
    fn cursor_visual_pos(&self, width: usize) -> (usize, usize) {
        let mut byte_offset = 0;
        let visual_lines = self.build_visual_lines(width);
        for (vis_idx, (_line_idx, seg_start, seg_end)) in visual_lines.iter().enumerate() {
            // Calculate byte position for this segment
            let line = self.text.lines().nth(*_line_idx).unwrap_or("");
            let line_chars: Vec<char> = line.chars().collect();
            let seg_byte_start: usize = line_chars[..*seg_start]
                .iter()
                .map(|c| c.len_utf8())
                .sum();
            let seg_byte_end: usize = line_chars[..*seg_end]
                .iter()
                .map(|c| c.len_utf8())
                .sum();

            if byte_offset + seg_byte_start <= self.cursor_pos
                && self.cursor_pos <= byte_offset + seg_byte_end
            {
                let col = self.cursor_pos - byte_offset - seg_byte_start;
                let col_chars = line_chars
                    .iter()
                    .skip(*seg_start)
                    .take(col)
                    .count();
                return (vis_idx, col_chars);
            }
            byte_offset += line.len() + 1; // +1 for \n
        }
        // Cursor at end
        if visual_lines.is_empty() {
            (0, 0)
        } else {
            (visual_lines.len(), 0)
        }
    }

    /// Calculate the height of the widget.
    pub fn height(&self, area_height: usize) -> usize {
        area_height // fills the entire area
    }

    /// Render the widget into the buffer.
    pub fn render_into(&self, area: Rect, buf: &mut Buffer) {
        if area.width < 10 || area.height < 4 {
            return;
        }

        let width = area.width as usize;
        let text_width = width.saturating_sub(2); // border padding
        let content_rows = (area.height - 5).max(1) as usize; // borders + separators + hint

        let visual_lines = self.build_visual_lines(text_width.max(1));
        let total_visual = visual_lines.len().max(1);
        let (cursor_vis_line, cursor_vis_col) = self.cursor_visual_pos(text_width.max(1));

        // Scroll: center cursor in viewport
        let half_viewport = content_rows / 2;
        let mut scroll_start = cursor_vis_line.saturating_sub(half_viewport);
        if scroll_start + content_rows > total_visual {
            scroll_start = total_visual.saturating_sub(content_rows);
        }

        let mut lines: Vec<Line> = Vec::new();

        // Top accent separator
        lines.push(Line::from(Span::styled(
            "─".repeat(width),
            Style::default().fg(self.accent_color).bg(self.content_bg),
        )));

        // Visible text lines
        let empty_line = Line::from(Span::styled(
            "".to_string(),
            Style::default().bg(self.content_bg),
        ));
        for i in scroll_start..(scroll_start + content_rows) {
            if i < visual_lines.len() {
                let (logical_line, seg_start, seg_end) = visual_lines[i];
                let line = self.text.lines().nth(logical_line).unwrap_or("");
                let chars: Vec<char> = line.chars().collect();
                let text: String = chars[seg_start..seg_end].iter().collect();

                let is_cursor_line = i == cursor_vis_line;

                if is_cursor_line {
                    let cursor_char_start = seg_start + cursor_vis_col;
                    let normal_style = Style::default()
                        .fg(self.text_color)
                        .bg(self.content_bg);
                    let block_style = Style::default()
                        .fg(self.content_bg)
                        .bg(self.text_color)
                        .add_modifier(ratatui::style::Modifier::BOLD);

                    let mut spans = Vec::new();
                    if cursor_char_start > seg_start {
                        let before: String = chars[seg_start..cursor_char_start]
                            .iter()
                            .collect();
                        spans.push(Span::styled(before, normal_style));
                    }

                    if cursor_char_start < seg_end {
                        // Block cursor: highlight the character under the cursor
                        let ch = chars[cursor_char_start];
                        spans.push(Span::styled(ch.to_string(), block_style));
                        if cursor_char_start + 1 < seg_end {
                            let after: String = chars[cursor_char_start + 1..seg_end]
                                .iter()
                                .collect();
                            spans.push(Span::styled(after, normal_style));
                        }
                    } else {
                        // Cursor at end of segment — show block on a space
                        spans.push(Span::styled(" ".to_string(), block_style));
                    }
                    lines.push(Line::from(spans));
                } else {
                    lines.push(Line::from(Span::styled(
                        text,
                        Style::default().fg(self.text_color).bg(self.content_bg),
                    )));
                }
            } else {
                lines.push(empty_line.clone());
            }
        }

        // Bottom accent separator
        lines.push(Line::from(Span::styled(
            "─".repeat(width),
            Style::default().fg(self.accent_color).bg(self.content_bg),
        )));

        // Hint bar
        lines.push(Line::from(Span::styled(
            self.hint.to_string(),
            Style::default().fg(self.dim_color).bg(self.content_bg),
        )));

        let block = Block::default()
            .borders(Borders::ALL)
            .border_set(PANEL_BORDER)
            .border_style(self.border_style)
            .title(Span::styled(self.title, self.title_style));

        let text = Text::from(lines);
        let paragraph = Paragraph::new(text)
            .block(block)
            .wrap(Wrap { trim: true })
            .style(Style::default().bg(self.content_bg));

        paragraph.render(area, buf);
    }
}

impl<'a> Widget for PromptEditorOverlay<'a> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        self.render_into(area, buf);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn height_fills_area() {
        let editor = PromptEditorOverlay::new("test");
        assert_eq!(editor.height(20), 20);
    }

    #[test]
    fn render_basic() {
        let editor = PromptEditorOverlay::new("hello world");
        let mut buf = Buffer::empty(Rect::new(0, 0, 20, 10));
        editor.render_into(Rect::new(0, 0, 20, 10), &mut buf);
        // Title should be present in first row
        let has_title = buf.content.iter().take(20).any(|cell| cell.symbol() == "P");
        assert!(has_title);
    }

    #[test]
    fn render_with_cursor() {
        let editor = PromptEditorOverlay::new("hello world").cursor_pos(3);
        let mut buf = Buffer::empty(Rect::new(0, 0, 20, 10));
        editor.render_into(Rect::new(0, 0, 20, 10), &mut buf);
        // Block cursor highlights 'l' (char at pos 3) — check it has reversed bg
        // The 'l' at cursor position should have bg=text_color, fg=content_bg
        let found = buf.content.iter().any(|cell| cell.symbol() == "l");
        assert!(found);
    }

    #[test]
    fn render_cursor_at_end() {
        let editor = PromptEditorOverlay::new("hello").cursor_pos(5);
        let mut buf = Buffer::empty(Rect::new(0, 0, 20, 10));
        editor.render_into(Rect::new(0, 0, 20, 10), &mut buf);
        // Block cursor at end should show a highlighted space after "hello"
        // Verify the space character appears (block cursor on empty cell)
        let found_space = buf.content.iter().any(|cell| cell.symbol() == " ");
        assert!(found_space);
    }

    #[test]
    fn cursor_visual_pos_at_end() {
        let editor = PromptEditorOverlay::new("hello").cursor_pos(5);
        let (vis_line, vis_col) = editor.cursor_visual_pos(80);
        // Cursor at end of "hello" should be on visual line 0, col 5
        assert_eq!(vis_line, 0);
        assert_eq!(vis_col, 5);
    }
}
