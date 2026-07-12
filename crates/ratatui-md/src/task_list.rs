//! Task list block rendering.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Paragraph, Widget, Wrap};

use crate::height::wrapped_height;
use crate::theme::MdTheme;
use crate::types::TaskListItem;

pub fn render(
    items: &[TaskListItem],
    area: Rect,
    buf: &mut Buffer,
    theme: &dyn MdTheme,
) {
    if area.width == 0 || area.height == 0 || items.is_empty() {
        return;
    }

    // Fill area with chat bg so trailing cells don't bleed through
    let bg_fill = Paragraph::new(Text::from(vec![])).style(theme.text_style());
    bg_fill.render(area, buf);

    let text_width = area.width as usize - 3; // "☑ " = ~3 chars
    let mut y_offset = 0u16;

    for item in items {
        if y_offset >= area.height {
            break;
        }

        let item_height = (wrapped_height(&item.text, text_width) + 1) as u16;
        let remaining = area.height - y_offset;
        let item_area = Rect {
            x: area.x,
            y: area.y + y_offset,
            width: area.width,
            height: item_height.min(remaining),
        };

        if item_area.height == 0 {
            continue;
        }

        let checkbox = if item.checked { "☑" } else { "☐" };

        let mut lines: Vec<Line> = Vec::new();
        for (li, line) in item.text.lines.iter().enumerate() {
            if li == 0 {
                let spans: Vec<Span> = vec![Span::styled(format!("{} ", checkbox), theme.list_bullet_style())]
                    .into_iter()
                    .chain(line.spans.iter().cloned())
                    .collect();
                lines.push(Line::from(spans));
            } else {
                let spans: Vec<Span> = vec![Span::styled("   ", theme.list_bullet_style())]
                    .into_iter()
                    .chain(line.spans.iter().cloned())
                    .collect();
                lines.push(Line::from(spans));
            }
        }
        if lines.is_empty() {
            lines.push(Line::from(Span::styled(
                format!("{} ", checkbox), theme.list_bullet_style()
            )));
        }

        let para = Paragraph::new(Text::from(lines)).wrap(Wrap { trim: true });
        para.render(item_area, buf);
        y_offset += item_height.min(remaining);
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
    fn render_checked_item() {
        let items = vec![TaskListItem { checked: true, text: Text::from("done") }];
        let area = Rect::new(0, 0, 20, 5);
        let mut buf = Buffer::empty(area);
        render(&items, area, &mut buf, &TestTheme);
        let got: String = buf.content.iter().map(|c| c.symbol().chars().next().unwrap_or(' ')).collect();
        assert!(got.contains("☑"), "Should have checked box");
        assert!(got.contains("done"));
    }

    #[test]
    fn render_unchecked_item() {
        let items = vec![TaskListItem { checked: false, text: Text::from("todo") }];
        let area = Rect::new(0, 0, 20, 5);
        let mut buf = Buffer::empty(area);
        render(&items, area, &mut buf, &TestTheme);
        let got: String = buf.content.iter().map(|c| c.symbol().chars().next().unwrap_or(' ')).collect();
        assert!(got.contains("☐"), "Should have unchecked box");
        assert!(got.contains("todo"));
    }

    #[test]
    fn render_mixed_checked_unchecked() {
        let items = vec![
            TaskListItem { checked: true, text: Text::from("done") },
            TaskListItem { checked: false, text: Text::from("todo") },
        ];
        let area = Rect::new(0, 0, 20, 10);
        let mut buf = Buffer::empty(area);
        render(&items, area, &mut buf, &TestTheme);
        let got: String = buf.content.iter().map(|c| c.symbol().chars().next().unwrap_or(' ')).collect();
        assert!(got.contains("☑"));
        assert!(got.contains("☐"));
    }

    #[test]
    fn render_empty_items() {
        let items: Vec<TaskListItem> = vec![];
        let area = Rect::new(0, 0, 20, 5);
        let mut buf = Buffer::empty(area);
        render(&items, area, &mut buf, &TestTheme);
        // Should not panic
    }

    #[test]
    fn render_item_with_empty_text() {
        let items = vec![TaskListItem { checked: true, text: Text::from(vec![]) }];
        let area = Rect::new(0, 0, 20, 5);
        let mut buf = Buffer::empty(area);
        render(&items, area, &mut buf, &TestTheme);
        let got: String = buf.content.iter().map(|c| c.symbol().chars().next().unwrap_or(' ')).collect();
        assert!(got.contains("☑"), "Should render checkbox even with empty text");
    }

    #[test]
    fn render_wrapped_text() {
        let long = "a".repeat(80);
        let items = vec![TaskListItem { checked: false, text: Text::from(long) }];
        let area = Rect::new(0, 0, 20, 10);
        let mut buf = Buffer::empty(area);
        render(&items, area, &mut buf, &TestTheme);
        // Should not panic — text wraps
    }

    #[test]
    fn render_zero_area() {
        let items = vec![TaskListItem { checked: false, text: Text::from("item") }];
        let area = Rect::new(0, 0, 0, 0);
        let mut buf = Buffer::empty(area);
        render(&items, area, &mut buf, &TestTheme);
    }

    #[test]
    fn render_clipped_area() {
        let items = vec![
            TaskListItem { checked: false, text: Text::from("item 1") },
            TaskListItem { checked: false, text: Text::from("item 2") },
            TaskListItem { checked: false, text: Text::from("item 3") },
        ];
        let area = Rect::new(0, 0, 20, 2); // Only room for 1 item
        let mut buf = Buffer::empty(area);
        render(&items, area, &mut buf, &TestTheme);
        // Should not panic — clips at area boundary
    }
}
