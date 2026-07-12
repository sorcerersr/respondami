//! Rule (horizontal rule) rendering.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Paragraph, Widget};

use crate::theme::MdTheme;

pub fn render(area: Rect, buf: &mut Buffer, theme: &dyn MdTheme) {
    // Fill entire area with bg first (covers wide-char continuation cells + blank row).
    let bg = Block::new().style(theme.text_style());
    bg.render(area, buf);

    // "─" is a wide character (2 cells), so we need half as many chars as width.
    // Odd widths get one trailing space.
    let num_chars = (area.width as usize).div_ceil(2);
    let hr = "─".repeat(num_chars);
    let style = theme.text_style().fg(theme.text_dim_color());
    let line = Line::from(Span::styled(hr, style));
    let para = Paragraph::new(line);
    para.render(area, buf);
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::style::{Color, Style, Modifier};

    struct TestTheme;
    impl crate::theme::MdTheme for TestTheme {
        fn text_style(&self) -> Style { Style::default().bg(Color::Rgb(33, 40, 48)) }
        fn heading_style(&self) -> Style { Style::default().add_modifier(Modifier::BOLD) }
        fn text_muted_color(&self) -> Color { Color::Gray }
        fn link_style(&self) -> Style { Style::default().fg(Color::Blue) }
        fn inline_code_style(&self) -> Style { Style::default().fg(Color::Cyan) }
        fn list_bullet_style(&self) -> Style { Style::default().fg(Color::Green) }
        fn code_block_style(&self) -> Style { Style::default().bg(Color::Rgb(30,30,30)) }
        fn text_dim_color(&self) -> Color { Color::DarkGray }
        fn text_muted_style(&self) -> Style { Style::default().fg(Color::DarkGray) }
    }

    #[test]
    fn render_rule_background_filled() {
        let area = Rect::new(0, 0, 10, 2);
        let mut buf = Buffer::empty(area);
        render(area, &mut buf, &TestTheme);
        // All cells should have a background set (none should be Reset/default terminal bg)
        for (i, cell) in buf.content.iter().enumerate() {
            assert_ne!(cell.bg, Color::Reset,
                "Cell index {} has no background — terminal bg bleeding through",
                i);
        }
    }

    #[test]
    fn render_rule_width_10() {
        let area = Rect::new(0, 0, 10, 1);
        let mut buf = Buffer::empty(area);
        render(area, &mut buf, &TestTheme);
        // "─" is wide (2 cells), so width=10 → 5 chars → 10 cells filled
        // 5 cells have "─", 5 continuation cells have " "
        assert_eq!(buf.content.len(), 10);
        let dash_cells: usize = buf.content.iter().filter(|c| c.symbol() == "─").count();
        assert_eq!(dash_cells, 5);
    }

    #[test]
    fn render_rule_width_1() {
        let area = Rect::new(0, 0, 1, 1);
        let mut buf = Buffer::empty(area);
        render(area, &mut buf, &TestTheme);
        // width=1 → div_ceil(1,2) = 1 char, truncated to area
        assert_eq!(buf.content.len(), 1);
    }

    #[test]
    fn render_rule_width_80() {
        let area = Rect::new(0, 0, 80, 1);
        let mut buf = Buffer::empty(area);
        render(area, &mut buf, &TestTheme);
        // width=80 → 40 wide chars → fills exactly 80 cells
        assert_eq!(buf.content.len(), 80);
        let dash_cells: usize = buf.content.iter().filter(|c| c.symbol() == "─").count();
        assert_eq!(dash_cells, 40);
    }
}
