//! Filled header bar widget.
//!
//! A bordered block with a filled background header row and content area.
//! Supports expanded/collapsed display modes.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Block, Borders, Paragraph, Widget, Wrap};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

/// A bordered block with a filled header row and content area.
///
/// Supports expanded/collapsed display modes. In collapsed mode,
/// the content is replaced with a single "▼ Output" line.
pub struct FilledHeaderBar<'a> {
    /// Text for the header row.
    header_text: &'a str,
    /// Background color of the header row (white text).
    header_color: ratatui::style::Color,
    /// Border color and style.
    border_color: ratatui::style::Color,
    /// Content lines to display in expanded mode.
    content: Vec<Line<'a>>,
    /// If true, show collapsed indicator instead of content.
    collapsed: bool,
    /// Background color of the content area.
    content_bg: ratatui::style::Color,
    /// Text color of the content area.
    content_fg: ratatui::style::Color,
    /// Collapsed indicator text.
    collapsed_text: &'a str,
    /// Collapsed indicator style.
    collapsed_style: Style,
}

impl<'a> FilledHeaderBar<'a> {
    /// Create a new `FilledHeaderBar` with the given header text.
    pub fn new(header_text: &'a str) -> Self {
        Self {
            header_text,
            header_color: ratatui::style::Color::Rgb(139, 92, 246),
            border_color: ratatui::style::Color::Rgb(139, 92, 246),
            content: Vec::new(),
            collapsed: false,
            content_bg: ratatui::style::Color::Rgb(30, 30, 46),
            content_fg: ratatui::style::Color::White,
            collapsed_text: "▼ Output",
            collapsed_style: Style::default().fg(ratatui::style::Color::Gray),
        }
    }

    /// Set the header background color.
    pub fn header_color(mut self, color: ratatui::style::Color) -> Self {
        self.header_color = color;
        self
    }

    /// Set the border color.
    pub fn border_color(mut self, color: ratatui::style::Color) -> Self {
        self.border_color = color;
        self
    }

    /// Set the content lines.
    pub fn content(mut self, content: Vec<Line<'a>>) -> Self {
        self.content = content;
        self
    }

    /// Set whether the widget is in collapsed mode.
    pub fn collapsed(mut self, collapsed: bool) -> Self {
        self.collapsed = collapsed;
        self
    }

    /// Set the background color of the content area.
    pub fn content_bg(mut self, color: ratatui::style::Color) -> Self {
        self.content_bg = color;
        self
    }

    /// Set the text color of the content area.
    pub fn content_fg(mut self, color: ratatui::style::Color) -> Self {
        self.content_fg = color;
        self
    }

    /// Set the collapsed indicator text.
    pub fn collapsed_text(mut self, text: &'a str) -> Self {
        self.collapsed_text = text;
        self
    }

    /// Set the collapsed indicator style.
    pub fn collapsed_style(mut self, style: Style) -> Self {
        self.collapsed_style = style;
        self
    }

    /// Calculate the height of the widget.
    pub fn height(&self, width: usize) -> usize {
        if width == 0 {
            return 2;
        }

        let header_height = 2; // top border + header row

        if self.collapsed {
            return header_height + 1; // collapsed indicator line
        }

        if self.content.is_empty() {
            return header_height + 1; // "[no output]" line
        }

        // Calculate wrapped lines
        let effective_width = width.saturating_sub(2); // border padding
        if effective_width == 0 {
            return header_height + self.content.len();
        }

        let wrapped: usize = self
            .content
            .iter()
            .map(|line| count_wrapped_lines(line, effective_width))
            .sum();
        header_height + wrapped
    }

    /// Render the widget into the buffer.
    pub fn render_into(&self, area: Rect, buf: &mut Buffer) {
        let width = area.width as usize;
        if width == 0 || area.height == 0 {
            return;
        }

        // Build content lines
        let mut content_lines: Vec<Line> = Vec::new();

        if self.collapsed {
            content_lines.push(Line::from(Span::styled(
                self.collapsed_text.to_string(),
                self.collapsed_style,
            )));
        } else if self.content.is_empty() {
            content_lines.push(Line::from(Span::styled(
                "[no output]".to_string(),
                self.collapsed_style,
            )));
        } else {
            content_lines = self.content.clone();
        }

        // Build block with colored border
        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(
                Style::default()
                    .fg(self.border_color)
                    .add_modifier(ratatui::style::Modifier::BOLD),
            );

        let content_text = Text::from(content_lines);
        let paragraph = Paragraph::new(content_text)
            .block(block)
            .wrap(Wrap { trim: true })
            .style(
                Style::default()
                    .fg(self.content_fg)
                    .bg(self.content_bg),
            );

        paragraph.render(area, buf);

        // Overwrite the top row with the filled header bar
        let header_style = Style::default()
            .fg(ratatui::style::Color::White)
            .bg(self.header_color);

        // Fill top row with header color background
        for x in 0..width {
            let mut cell = ratatui::buffer::Cell::default();
            cell.set_symbol(" ");
            cell.set_style(header_style);
            buf[(((area.x) + x as u16), area.y)] = cell;
        }

        // Write header text starting from column 1 (inside left border).
        // Track display column position so wide characters (emoji, CJK) don't
        // cause subsequent characters to overwrite the combining half-glyph.
        // Replace control characters (\n, \r) with spaces to prevent terminal
        // from interpreting them as line breaks, which would spill header text
        // onto content rows.
        let mut col = 1;
        for c in self.header_text.chars() {
            let ch = if c == '\n' || c == '\r' { ' ' } else { c };
            let ch_width = ch.width().unwrap_or(1);
            if col + ch_width - 1 < width {
                let mut cell = ratatui::buffer::Cell::default();
                cell.set_symbol(ch.to_string().as_str());
                cell.set_style(header_style);
                buf[((area.x + (col) as u16), area.y)] = cell;
            }
            col += ch_width;
        }
    }
}

/// Count how many wrapped lines a single `Line` produces at the given display width.
///
/// Uses grapheme-by-grapheme iteration to respect character boundaries, unlike
/// simple ceiling division which underestimates when wide characters sit at wrap points.
fn count_wrapped_lines(line: &Line, max_width: usize) -> usize {
    let total_width = line.width();
    if total_width == 0 || max_width == 0 {
        return 1;
    }

    let mut count = 1;
    let mut current_width = 0;

    for grapheme in line.styled_graphemes(Style::default()) {
        let gw = grapheme.symbol.width();
        if current_width > 0 && current_width + gw > max_width {
            // Wrap: does this grapheme fit on a fresh line?
            if gw > max_width {
                // Grapheme wider than line — it takes its own line regardless.
                count += 1;
                current_width = 0;
            } else {
                count += 1;
                current_width = gw;
            }
        } else {
            current_width += gw;
        }
    }

    count
}

impl<'a> Widget for FilledHeaderBar<'a> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        self.render_into(area, buf);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::buffer::Buffer;
    use ratatui::layout::Rect;

    #[test]
    fn height_expanded_empty_content() {
        let widget = FilledHeaderBar::new("Test Header");
        assert_eq!(widget.height(80), 3); // 2 (header) + 1 (no output)
    }

    #[test]
    fn height_collapsed() {
        let widget = FilledHeaderBar::new("Test Header").collapsed(true);
        assert_eq!(widget.height(80), 3); // 2 (header) + 1 (collapsed indicator)
    }

    #[test]
    fn height_expanded_with_content() {
        let widget = FilledHeaderBar::new("Test Header").content(vec![
            Line::from("line 1"),
            Line::from("line 2"),
            Line::from("line 3"),
        ]);
        assert_eq!(widget.height(80), 5); // 2 (header) + 3 (content)
    }

    #[test]
    fn height_zero_width() {
        let widget = FilledHeaderBar::new("Test Header");
        assert_eq!(widget.height(0), 2);
    }

    #[test]
    fn render_collapsed() {
        let widget = FilledHeaderBar::new("Skill Activation")
            .header_color(ratatui::style::Color::Rgb(139, 92, 246))
            .collapsed(true);
        let mut buf = Buffer::empty(Rect::new(0, 0, 20, 3));
        widget.render_into(Rect::new(0, 0, 20, 3), &mut buf);
        // Verify header row has filled background
        assert_eq!(buf[(1, 0)].symbol(), "S"); // "Skill Activation"
        assert_eq!(buf[(1, 0)].bg, ratatui::style::Color::Rgb(139, 92, 246));
    }

    #[test]
    fn render_expanded() {
        let widget = FilledHeaderBar::new("Read: src/main.rs")
            .content(vec![Line::from("fn main() {")]);
        let mut buf = Buffer::empty(Rect::new(0, 0, 20, 3));
        widget.render_into(Rect::new(0, 0, 20, 3), &mut buf);
        // Header row
        assert_eq!(buf[(1, 0)].symbol(), "R");
        // Content row
        assert_eq!(buf[(1, 1)].symbol(), "f"); // "fn main() {"
    }

    #[test]
    fn render_empty_content_shows_no_output() {
        let widget = FilledHeaderBar::new("Test").content(vec![]);
        let mut buf = Buffer::empty(Rect::new(0, 0, 20, 3));
        widget.render_into(Rect::new(0, 0, 20, 3), &mut buf);
        // Content row
        assert_eq!(buf[(1, 1)].symbol(), "[");
    }

    // ---------------------------------------------------------------------------
    // Regression: wide characters (emoji, CJK, box-drawing) in height + render
    // ---------------------------------------------------------------------------

    #[test]
    fn count_wrapped_lines_wide_chars_at_boundary() {
        // "ab😀cd" = a(1) + b(1) + 😀(2) + c(1) + d(1) = 6 display width
        // At max_width=3: "ab" (2), "😀" (2) wraps to line 2, "c" (1) fits, "d" (1) wraps to line 3
        // = 3 lines
        let line = Line::from("ab😀cd");
        assert_eq!(count_wrapped_lines(&line, 3), 3);
    }

    #[test]
    fn count_wrapped_lines_wide_chars_fit() {
        // "ab😀cd" = 6 display width, max_width=6 → 1 line
        let line = Line::from("ab😀cd");
        assert_eq!(count_wrapped_lines(&line, 6), 1);
    }

    #[test]
    fn count_wrapped_lines_wider_than_line() {
        // "😀" = 2 display width, max_width=1 → 😀 doesn't fit but takes own line
        // = 1 line (the emoji alone)
        let line = Line::from("😀");
        assert_eq!(count_wrapped_lines(&line, 1), 1);
    }

    #[test]
    fn count_wrapped_lines_box_drawing() {
        // "┌────┐" = 6 display width, max_width=3
        // "┌──" (3) → line 1, "─┐" (3) → line 2
        // Wait: ┌(1) + ─(1) + ─(1) = 3 → line 1, ─(1) + ─(1) + ┐(1) = 3 → line 2
        let line = Line::from("┌────┐");
        assert_eq!(count_wrapped_lines(&line, 3), 2);
    }

    #[test]
    fn height_wide_chars_content() {
        // Content with emoji: "a😀b" (4 display width), max_width=2 (effective)
        // Wraps to 3 lines: "a" (1), "😀" (2), "b" (1)
        let widget = FilledHeaderBar::new("T").content(vec![Line::from("a😀b")]);
        assert_eq!(widget.height(4), 5); // 2 (header) + 3 (wrapped content)
    }

    #[test]
    fn render_header_wide_chars_no_overlap() {
        // Header "😀test" — 😀 takes 2 cols, 't' should start at col 3 (not col 2)
        let widget = FilledHeaderBar::new("😀test");
        let mut buf = Buffer::empty(Rect::new(0, 0, 10, 3));
        widget.render_into(Rect::new(0, 0, 10, 3), &mut buf);
        // Col 0 = border, col 1-2 = 😀 (2 display cols), col 3 = 't'
        assert_eq!(buf[(1, 0)].symbol(), "😀");
        assert_eq!(buf[(3, 0)].symbol(), "t");
        assert_eq!(buf[(4, 0)].symbol(), "e");
    }

    #[test]
    fn render_header_box_drawing_no_overlap() {
        // Header "┌─┐x" — each box char is 1 display width
        let widget = FilledHeaderBar::new("┌─┐x");
        let mut buf = Buffer::empty(Rect::new(0, 0, 10, 3));
        widget.render_into(Rect::new(0, 0, 10, 3), &mut buf);
        assert_eq!(buf[(1, 0)].symbol(), "┌");
        assert_eq!(buf[(2, 0)].symbol(), "─");
        assert_eq!(buf[(3, 0)].symbol(), "┐");
        assert_eq!(buf[(4, 0)].symbol(), "x");
    }

    // ---------------------------------------------------------------------------
    // Regression: control characters (newlines) in header text
    // ---------------------------------------------------------------------------

    #[test]
    fn render_header_with_newlines_does_not_break_layout() {
        // Multi-line command (e.g. git commit with message body) produces newlines
        // in header text. These must NOT be written as cell symbols — they cause
        // the terminal to interpret them as line breaks, spilling header text
        // onto content rows.
        let widget = FilledHeaderBar::new("bash: rtk git commit -m \"fix\n\nbody text here\"")
            .content(vec![Line::from("content line 1")]);
        let mut buf = Buffer::empty(Rect::new(0, 0, 40, 4));
        widget.render_into(Rect::new(0, 0, 40, 4), &mut buf);

        // Row 0 = header row. Should start with 'b' (from "bash").
        assert_eq!(buf[(1, 0)].symbol(), "b");

        // Row 1 = content row. Should contain "content line 1", NOT header text.
        assert_eq!(buf[(1, 1)].symbol(), "c");

        // Verify no newline characters leaked into the buffer as symbols
        for y in 0..4u16 {
            for x in 0..40u16 {
                let sym = buf[(x, y)].symbol();
                assert!(
                    sym != "\n" && sym != "\r",
                    "Control character leaked into buffer at ({}, {}): {:?}",
                    x, y, sym
                );
            }
        }
    }

    #[test]
    fn render_header_newlines_replaced_with_spaces() {
        // Newlines in header text should be replaced with spaces so the text
        // remains readable on a single line.
        // "line1\nline2\r\nline3" → "line1 line2  line3" (\r\n → two spaces)
        let widget = FilledHeaderBar::new("line1\nline2\r\nline3");
        let mut buf = Buffer::empty(Rect::new(0, 0, 30, 3));
        widget.render_into(Rect::new(0, 0, 30, 3), &mut buf);

        // All text should be on row 0, with spaces where newlines were.
        assert_eq!(buf[(1, 0)].symbol(), "l"); // 'l' from "line1"
        // col 1-5: line1, col 6: space (was \n), col 7: 'l' from "line2"
        assert_eq!(buf[(7, 0)].symbol(), "l");
        // Row 1 should be the content area, not header text
        assert_eq!(buf[(1, 1)].symbol(), "["); // "[no output]"
    }

    #[test]
    fn render_header_only_newlines() {
        // Edge case: header is only newlines
        let widget = FilledHeaderBar::new("\n\n\n");
        let mut buf = Buffer::empty(Rect::new(0, 0, 10, 3));
        widget.render_into(Rect::new(0, 0, 10, 3), &mut buf);

        // Row 0 should be filled with spaces (newlines → spaces)
        for x in 0..10u16 {
            assert_eq!(buf[(x, 0)].symbol(), " ", "Unexpected char at col {}: {:?}", x, buf[(x, 0)].symbol());
        }
    }

    // ---------------------------------------------------------------------------
    // Regression: wide characters in content (ceiling division)
    // ---------------------------------------------------------------------------

    #[test]
    fn height_ceiling_division_vs_actual() {
        // Regression: ceiling division gave wrong answer for wide chars
        // "a😀b" (4 display width), effective_width=2
        // Ceiling: 4.div_ceil(2) = 2 (WRONG)
        // Actual: "a"(1), "😀"(2), "b"(1) = 3 lines (CORRECT)
        let widget = FilledHeaderBar::new("T").content(vec![Line::from("a😀b")]);
        let h = widget.height(4); // effective_width = 4 - 2 = 2
        assert_eq!(h, 5); // 2 (header) + 3 (actual wrapped lines)
    }
}
