//! Block quote rendering.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::Text;
use ratatui::widgets::{Block, Borders, Paragraph, Widget, Wrap};

use crate::theme::MdTheme;

pub fn render(text: &Text<'static>, area: Rect, buf: &mut Buffer, theme: &dyn MdTheme) {
    let block = Block::new()
        .borders(Borders::LEFT)
        .border_style(Style::default().fg(theme.text_dim_color()));
    let inner = block.inner(area);
    block.render(area, buf);
    if inner.width > 0 && inner.height > 0 {
        let para = Paragraph::new(text.clone())
            .style(theme.text_muted_style())
            .wrap(Wrap { trim: true });
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
    fn render_basic_quote() {
        let text = Text::from("quoted");
        let area = Rect::new(0, 0, 15, 3);
        let mut buf = Buffer::empty(area);
        render(&text, area, &mut buf, &TestTheme);
        let got: String = buf.content.iter().map(|c| c.symbol().chars().next().unwrap_or(' ')).collect();
        assert!(got.contains("quoted"));
    }

    #[test]
    fn render_multiline_quote() {
        let text = Text::from(vec![
            ratatui::text::Line::from("line one"),
            ratatui::text::Line::from("line two"),
        ]);
        let area = Rect::new(0, 0, 20, 5);
        let mut buf = Buffer::empty(area);
        render(&text, area, &mut buf, &TestTheme);
        let got: String = buf.content.iter().map(|c| c.symbol().chars().next().unwrap_or(' ')).collect();
        assert!(got.contains("line one"));
        assert!(got.contains("line two"));
    }

    #[test]
    fn render_zero_area() {
        let text = Text::from("quote");
        let area = Rect::new(0, 0, 0, 0);
        let mut buf = Buffer::empty(area);
        render(&text, area, &mut buf, &TestTheme);
    }
}
