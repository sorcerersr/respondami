//! Heading block rendering.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::text::Text;
use ratatui::widgets::{Paragraph, Widget, Wrap};

use crate::theme::MdTheme;

pub fn render(text: &Text<'static>, area: Rect, buf: &mut Buffer, theme: &dyn MdTheme) {
    let para = Paragraph::new(text.clone())
        .style(theme.heading_style())
        .wrap(Wrap { trim: true });
    para.render(area, buf);
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::buffer::Buffer;
    use ratatui::layout::Rect;
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
    fn render_basic_heading() {
        let text = Text::from("Title");
        let area = Rect::new(0, 0, 10, 2);
        let mut buf = Buffer::empty(area);
        render(&text, area, &mut buf, &TestTheme);
        let got: String = buf.content.iter().map(|c| c.symbol().chars().next().unwrap_or(' ')).collect();
        assert!(got.starts_with("Title"));
    }

    #[test]
    fn render_heading_zero_area() {
        let text = Text::from("Title");
        let area = Rect::new(0, 0, 0, 0);
        let mut buf = Buffer::empty(area);
        render(&text, area, &mut buf, &TestTheme);
    }
}
