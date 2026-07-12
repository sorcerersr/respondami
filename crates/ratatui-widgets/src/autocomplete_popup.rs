//! Autocomplete popup widget.
//!
//! A dropdown list that can be centered above the input row or anchored near a
//! cursor column. Supports virtual scrolling, front-truncation of long items,
//! and an optional footer row with a divider.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Block, Borders, Paragraph, Widget};
use unicode_width::UnicodeWidthStr;
use unicode_width::UnicodeWidthChar;

use super::panel_overlay::PANEL_BORDER;

/// A dropdown list for autocomplete popups.
pub struct AutocompletePopup {
    /// Items to display.
    items: Vec<String>,
    /// Index of the selected item.
    selected: usize,
    /// Virtual scroll offset.
    scroll_offset: usize,
    /// Column where the popup should be anchored (x position). Used when `centered` is false.
    anchor_col: u16,
    /// Row where the popup grows upward from (y position). Used when `centered` is false.
    anchor_row: u16,
    /// Maximum height of the popup.
    max_height: usize,
    /// Minimum width of the popup.
    min_width: usize,
    /// Maximum width of the popup.
    max_width: usize,
    /// When true, the popup uses the full provided area (position computed by caller).
    centered: bool,
    /// When true, truncate items from the front with `…` if they exceed `truncate_max_width`.
    truncate_front: bool,
    /// Maximum display width before front-truncation kicks in.
    truncate_max_width: usize,
    /// Optional footer line rendered below the item list, separated by a divider.
    footer: Option<Line<'static>>,
    /// Border style.
    border_style: Style,
    /// Title style.
    title_style: Style,
    /// Content background color.
    content_bg: ratatui::style::Color,
    /// Text color.
    text_color: ratatui::style::Color,
    /// Accent color (for selected bg).
    accent_color: ratatui::style::Color,
    /// Title text.
    title: String,
}

impl Default for AutocompletePopup {
    fn default() -> Self {
        Self::new()
    }
}

impl AutocompletePopup {
    /// Create a new `AutocompletePopup`.
    pub fn new() -> Self {
        Self {
            items: Vec::new(),
            selected: 0,
            scroll_offset: 0,
            anchor_col: 0,
            anchor_row: 0,
            max_height: 10,
            min_width: 20,
            max_width: 50,
            centered: false,
            truncate_front: false,
            truncate_max_width: 60,
            footer: None,
            border_style: Style::default().fg(ratatui::style::Color::Rgb(0x6c, 0xb6, 0xff)),
            title_style: Style::default()
                .fg(ratatui::style::Color::Rgb(0x6c, 0xb6, 0xff))
                .bg(ratatui::style::Color::Rgb(0x2a, 0x31, 0x3c))
                .add_modifier(ratatui::style::Modifier::BOLD)
                .add_modifier(ratatui::style::Modifier::REVERSED),
            content_bg: ratatui::style::Color::Rgb(0x2a, 0x31, 0x3c),
            text_color: ratatui::style::Color::Rgb(0xd1, 0xd7, 0xe0),
            accent_color: ratatui::style::Color::Rgb(0x47, 0x8b, 0xe6),
            title: " Files ".to_string(),
        }
    }

    /// Set the items to display.
    pub fn items(mut self, items: Vec<String>) -> Self {
        self.items = items;
        self
    }

    /// Set the selected item index.
    pub fn selected(mut self, selected: usize) -> Self {
        self.selected = selected;
        self
    }

    /// Set the virtual scroll offset.
    pub fn scroll_offset(mut self, offset: usize) -> Self {
        self.scroll_offset = offset;
        self
    }

    /// Set the anchor column (x position).
    pub fn anchor_col(mut self, col: u16) -> Self {
        self.anchor_col = col;
        self
    }

    /// Set the anchor row (y position).
    pub fn anchor_row(mut self, row: u16) -> Self {
        self.anchor_row = row;
        self
    }

    /// Set the maximum height.
    pub fn max_height(mut self, height: usize) -> Self {
        self.max_height = height;
        self
    }

    /// Set the minimum width.
    pub fn min_width(mut self, width: usize) -> Self {
        self.min_width = width;
        self
    }

    /// Set the maximum width.
    pub fn max_width(mut self, width: usize) -> Self {
        self.max_width = width;
        self
    }

    /// Set centered mode. When true, the popup uses the full provided area.
    pub fn centered(mut self, centered: bool) -> Self {
        self.centered = centered;
        self
    }

    /// Set front-truncation mode. When true, truncate items from front with `…`.
    pub fn truncate_front(mut self, truncate: bool) -> Self {
        self.truncate_front = truncate;
        self
    }

    /// Set the maximum display width before front-truncation kicks in.
    pub fn truncate_max_width(mut self, width: usize) -> Self {
        self.truncate_max_width = width;
        self
    }

    /// Set an optional footer line rendered below the item list.
    pub fn footer(mut self, footer: Line<'static>) -> Self {
        self.footer = Some(footer);
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

    /// Calculate the area for the popup given the terminal dimensions.
    ///
    /// Only used when `centered` is false. When centered, the caller provides
    /// the area directly.
    pub fn area(&self, terminal_width: u16, _terminal_height: u16) -> Option<Rect> {
        if self.items.is_empty() {
            return None;
        }

        let visible_count = self.items.len().min(self.max_height);
        let footer_height = if self.footer.is_some() { 2 } else { 0 }; // divider + footer
        let popup_height = (visible_count + 2 + footer_height) as u16;

        // Content-based width
        let longest = self
            .items
            .iter()
            .map(|item| item.len())
            .max()
            .unwrap_or(self.min_width);
        let popup_width = (longest + 2)
            .clamp(self.min_width, self.max_width) as u16;

        // Horizontal positioning near the anchor column
        let mut popup_x = self.anchor_col;
        if popup_x + popup_width > terminal_width {
            popup_x = terminal_width.saturating_sub(popup_width);
        }
        let popup_x = popup_x.min(terminal_width.saturating_sub(popup_width));

        // Vertical: grows upward from anchor row
        let space_above = self.anchor_row;
        let popup_height = popup_height.min(space_above);
        if popup_height < 2 {
            return None;
        }
        let popup_y = self.anchor_row - popup_height;

        let popup_area = Rect {
            x: popup_x,
            y: popup_y,
            width: popup_width,
            height: popup_height,
        };

        Some(popup_area)
    }

    /// Calculate the height of the widget.
    pub fn height(&self) -> usize {
        let visible = self.items.len().min(self.max_height);
        let footer_height = if self.footer.is_some() { 2 } else { 0 };
        visible + 2 + footer_height // borders + footer (divider + line)
    }

    /// Truncate a string from the front if it exceeds `max_width`, preserving
    /// the rightmost characters. Prepends `…` to indicate truncation.
    fn truncate_front_str(s: &str, max_width: usize) -> String {
        let display_width = s.width();
        if display_width <= max_width {
            return s.to_string();
        }

        // Need at least 1 char for ellipsis + 1 char for content
        let budget = max_width.max(2);
        let content_budget = budget - 1; // -1 for ellipsis

        // Keep last N display-width chars from the right
        let mut kept_width = 0;
        let mut keep_byte_idx = s.len();
        for ch in s.chars().rev() {
            let ch_width = ch.width().unwrap_or(1);
            if kept_width + ch_width > content_budget {
                break;
            }
            kept_width += ch_width;
            keep_byte_idx -= ch.len_utf8();
        }

        format!("…{}", &s[keep_byte_idx..])
    }

    /// Render the widget into the buffer.
    pub fn render_into(&self, area: Rect, buf: &mut Buffer) {
        if area.width < 4 || area.height < 2 {
            return;
        }

        let has_footer = self.footer.is_some();
        // Content area: area.height - 2 (top/bottom borders) - 1 (footer) - 1 (divider)
        let content_rows = if has_footer {
            (area.height - 4).max(1) as usize
        } else {
            (area.height - 2).max(1) as usize
        };
        let page_size = content_rows.min(self.max_height);

        // Virtual scroll: compute visible window
        let mut offset = self.scroll_offset;
        if self.selected >= offset + page_size {
            offset = self.selected - page_size + 1;
        } else if self.selected < offset {
            offset = self.selected;
        }
        let visible_end = (offset + page_size).min(self.items.len());

        let inner_width = (area.width - 2).max(1) as usize;
        let mut lines: Vec<Line> = Vec::new();
        for (i, item) in self.items[offset..visible_end].iter().enumerate() {
            let is_selected = (offset + i) == self.selected;
            let arrow = if is_selected { "→ " } else { "  " };

            let display_item = if self.truncate_front && item.width() > inner_width.saturating_sub(2) {
                Self::truncate_front_str(item, inner_width.saturating_sub(2))
            } else {
                item.clone()
            };

            let display = format!("{}{}", arrow, display_item);

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
        while lines.len() < content_rows {
            lines.push(Line::from(Span::styled(
                "".to_string(),
                Style::default().bg(self.content_bg),
            )));
        }

        // Divider + footer (rendered as part of the content lines)
        if has_footer {
            // Divider line: ─ characters spanning the inner width
            let divider_chars = "─".repeat(inner_width);
            lines.push(Line::from(Span::styled(
                divider_chars,
                Style::default().fg(self.border_style.fg.unwrap_or(self.text_color)).bg(self.content_bg),
            )));
            lines.push(self.footer.clone().unwrap());
        }

        let block = Block::default()
            .borders(Borders::ALL)
            .border_set(PANEL_BORDER)
            .border_style(self.border_style)
            .title(Span::styled(&self.title, self.title_style));

        let text = Text::from(lines);
        let paragraph = Paragraph::new(text)
            .block(block)
            .style(Style::default().bg(self.content_bg));

        paragraph.render(area, buf);
    }
}

impl Widget for AutocompletePopup {
    fn render(self, area: Rect, buf: &mut Buffer) {
        self.render_into(area, buf);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn area_empty_items() {
        let popup = AutocompletePopup::new();
        assert!(popup.area(80, 24).is_none());
    }

    #[test]
    fn area_with_items() {
        let popup = AutocompletePopup::new()
            .items(vec!["file1.rs".into(), "file2.rs".into()])
            .anchor_col(10)
            .anchor_row(20);
        let area = popup.area(80, 24);
        assert!(area.is_some());
        let a = area.unwrap();
        assert_eq!(a.x, 10);
        assert_eq!(a.y, 16); // 20 - 4 (2 items + 2 borders)
    }

    #[test]
    fn height_minimum() {
        let popup = AutocompletePopup::new();
        assert_eq!(popup.height(), 2); // 0 items + 2 borders
    }

    #[test]
    fn height_with_items() {
        let popup = AutocompletePopup::new().items(vec!["a".into(), "b".into(), "c".into()]);
        assert_eq!(popup.height(), 5); // 3 items + 2 borders
    }

    #[test]
    fn height_with_footer() {
        let popup = AutocompletePopup::new()
            .items(vec!["a".into(), "b".into()])
            .footer(Line::from("footer"));
        assert_eq!(popup.height(), 6); // 2 items + 2 borders + 2 footer
    }

    #[test]
    fn render_basic() {
        let popup = AutocompletePopup::new()
            .items(vec!["file1.rs".into(), "file2.rs".into()])
            .selected(0);
        let area = Rect::new(0, 0, 15, 4);
        let mut buf = Buffer::empty(area);
        popup.render_into(area, &mut buf);
        // First item should have arrow
        assert_eq!(buf[(1, 1)].symbol(), "→");
    }

    #[test]
    fn render_with_footer() {
        let popup = AutocompletePopup::new()
            .items(vec!["file1.rs".into(), "file2.rs".into()])
            .selected(0)
            .footer(Line::from(Span::styled("toggle hidden", Style::default().fg(ratatui::style::Color::DarkGray))));
        let area = Rect::new(0, 0, 20, 6); // 2 items + 2 borders + 2 footer = 6
        let mut buf = Buffer::empty(area);
        popup.render_into(area, &mut buf);
        // Arrow on first item
        assert_eq!(buf[(1, 1)].symbol(), "→");
        // Divider row (row index 3 = items row 2, but we have 2 items so divider at row 3)
        // Footer at row 4
        assert_eq!(buf[(1, 4)].symbol(), "t");
    }

    #[test]
    fn truncate_front_str_short() {
        let result = AutocompletePopup::truncate_front_str("short", 80);
        assert_eq!(result, "short");
    }

    #[test]
    fn truncate_front_str_long() {
        let result = AutocompletePopup::truncate_front_str("src/components/slider.rs", 15);
        assert!(result.contains("…"));
        assert!(result.ends_with("slider.rs"));
        assert_eq!(result.width(), 15);
    }

    #[test]
    fn truncate_front_str_wide_chars() {
        let result = AutocompletePopup::truncate_front_str("📁 src/main.rs", 12);
        assert!(result.contains("…"));
        // Width should be ≤ 12
        assert!(result.width() <= 12);
    }

    #[test]
    fn render_truncation_enabled() {
        let popup = AutocompletePopup::new()
            .items(vec!["src/components/ui/widgets/slider.rs".into()])
            .selected(0)
            .truncate_front(true)
            .truncate_max_width(20);
        let area = Rect::new(0, 0, 25, 3);
        let mut buf = Buffer::empty(area);
        popup.render_into(area, &mut buf);
        // Should have arrow and ellipsis
        assert_eq!(buf[(1, 1)].symbol(), "→");
        // Content should contain ellipsis
        let row_text: String = (0..24).map(|x| buf.cell((x, 1)).map(|c| c.symbol()).unwrap_or("")) .collect();
        assert!(row_text.contains("…"), "Expected ellipsis in rendered row, got: {}", row_text);
    }
}
