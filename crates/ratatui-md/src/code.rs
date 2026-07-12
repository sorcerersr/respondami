//! Code block rendering.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Block, Borders, Paragraph, Widget, Wrap};

use crate::theme::MdTheme;

pub fn render(lang: &str, content: &str, area: Rect, buf: &mut Buffer, theme: &dyn MdTheme) {
    let border_style = Style::default().fg(theme.text_dim_color());
    let block = Block::new()
        .borders(Borders::TOP | Borders::BOTTOM)
        .border_style(border_style);
    let inner = block.inner(area);

    // Fill with dark bg (covers sides where no border exists)
    let bg_fill = Paragraph::new(Text::from(vec![])).style(theme.code_block_style());
    bg_fill.render(area, buf);

    block.render(area, buf);

    if inner.width > 0 && inner.height > 0 {
        let title = if lang.is_empty() { "Code" } else { lang };
        let mut lines: Vec<Line> = vec![
            Line::from(Span::styled(format!(" {} ", title), Style::default().fg(theme.text_dim_color()))),
        ];
        lines.extend(content.lines().map(|line| {
            Line::from(Span::styled(line.to_string(), theme.inline_code_style()))
        }));
        let text = Text::from(lines);
        let para = Paragraph::new(text).wrap(Wrap { trim: false });
        para.render(inner, buf);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::style::{Color, Style, Modifier};

    struct TestTheme;
    impl crate::theme::MdTheme for TestTheme {
        fn text_style(&self) -> Style { Style::default() }
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
    fn render_with_language() {
        let area = Rect::new(0, 0, 20, 5);
        let mut buf = Buffer::empty(area);
        render("rust", "fn main() {}", area, &mut buf, &TestTheme);
        let got: String = buf.content.iter().map(|c| c.symbol().chars().next().unwrap_or(' ')).collect();
        assert!(got.contains("rust"), "Should contain language title");
        assert!(got.contains("fn main"), "Should contain code");
    }

    #[test]
    fn render_empty_language_shows_code() {
        let area = Rect::new(0, 0, 20, 5);
        let mut buf = Buffer::empty(area);
        render("", "let x = 1;", area, &mut buf, &TestTheme);
        let got: String = buf.content.iter().map(|c| c.symbol().chars().next().unwrap_or(' ')).collect();
        assert!(got.contains("Code"), "Should show 'Code' for empty language");
    }

    #[test]
    fn render_multiline_content() {
        let area = Rect::new(0, 0, 20, 8);
        let mut buf = Buffer::empty(area);
        render("python", "def foo():\n    return 1", area, &mut buf, &TestTheme);
        let got: String = buf.content.iter().map(|c| c.symbol().chars().next().unwrap_or(' ')).collect();
        assert!(got.contains("def foo"));
        assert!(got.contains("return 1"));
    }

    #[test]
    fn render_empty_content() {
        let area = Rect::new(0, 0, 20, 4);
        let mut buf = Buffer::empty(area);
        render("txt", "", area, &mut buf, &TestTheme);
        let got: String = buf.content.iter().map(|c| c.symbol().chars().next().unwrap_or(' ')).collect();
        assert!(got.contains("txt"), "Should still show language title");
    }

    #[test]
    fn render_small_area() {
        let area = Rect::new(0, 0, 2, 2);
        let mut buf = Buffer::empty(area);
        render("rust", "code", area, &mut buf, &TestTheme);
        // Should not panic
    }

    #[test]
    fn render_zero_area() {
        let area = Rect::new(0, 0, 0, 0);
        let mut buf = Buffer::empty(area);
        render("rust", "code", area, &mut buf, &TestTheme);
        // Should not panic — inner area will be 0
    }
}
