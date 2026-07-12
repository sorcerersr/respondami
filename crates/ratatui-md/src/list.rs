//! List block rendering (ordered and unordered, with nesting).

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Paragraph, Widget, Wrap};

use crate::height::wrapped_height;
use crate::theme::MdTheme;
use crate::types::ListItem;

/// Render a list recursively (handles nesting).
/// `indent` is the nesting depth (0 for top-level).
pub fn render(
    items: &[ListItem],
    start: u32,
    ordered: bool,
    area: Rect,
    buf: &mut Buffer,
    theme: &dyn MdTheme,
) {
    render_inner(items, start, ordered, 0, area, buf, theme);
}

fn render_inner(
    items: &[ListItem],
    start: u32,
    ordered: bool,
    indent: usize,
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

    let prefix = "  ".repeat(indent);
    let bullet_width = prefix.chars().count() + if ordered { 4 } else { 2 };
    let text_width = area.width as usize - bullet_width.max(1);

    let mut y_offset = 0u16;

    for (i, item) in items.iter().enumerate() {
        if y_offset >= area.height {
            break;
        }

        let item_height = list_item_text_height(item, text_width) as u16;
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

        let bullet = if ordered {
            format!("{}{}. ", prefix, start + i as u32)
        } else {
            format!("{}• ", prefix)
        };

        // Build lines: first line has bullet, continuation lines have indent
        let mut lines: Vec<Line> = Vec::new();
        for (li, line) in item.text.lines.iter().enumerate() {
            if li == 0 {
                let spans: Vec<Span> = vec![Span::styled(bullet.clone(), theme.list_bullet_style())]
                    .into_iter()
                    .chain(line.spans.iter().cloned())
                    .collect();
                lines.push(Line::from(spans));
            } else {
                let spans: Vec<Span> = vec![Span::styled(format!("{}  ", prefix), theme.list_bullet_style())]
                    .into_iter()
                    .chain(line.spans.iter().cloned())
                    .collect();
                lines.push(Line::from(spans));
            }
        }
        if lines.is_empty() {
            lines.push(Line::from(Span::styled(bullet.clone(), theme.list_bullet_style())));
        }

        let para = Paragraph::new(Text::from(lines)).wrap(Wrap { trim: true });
        para.render(item_area, buf);
        y_offset += item_height.min(remaining);

        // Render children
        if !item.children.is_empty() && y_offset < area.height {
            let children_height: usize = item.children.iter()
                .map(|c| list_item_render_height(c, text_width))
                .sum();
            let remaining = (area.height - y_offset) as usize;
            let children_area = Rect {
                x: area.x,
                y: area.y + y_offset,
                width: area.width,
                height: children_height.min(remaining) as u16,
            };
            render_inner(&item.children, 1, false, indent + 1, children_area, buf, theme);
            y_offset += children_height.min(remaining) as u16;
        }
    }
}

/// Height of a list item's own text (excluding children).
/// Used for rendering — parent text gets its own space, then children follow.
fn list_item_text_height(item: &ListItem, text_width: usize) -> usize {
    if text_width == 0 {
        return 1;
    }
    // Empty items render as just the bullet (1 line), not bullet + empty paragraph (2 lines).
    if item.text.lines.is_empty() {
        1
    } else {
        wrapped_height(&item.text, text_width) + 1
    }
}

/// Height of a list item including its text and children.
/// Used for total height calculation (e.g., for height() queries).
fn list_item_render_height(item: &ListItem, text_width: usize) -> usize {
    let text_h = list_item_text_height(item, text_width);
    let children_h: usize = item.children.iter()
        .map(|c| list_item_render_height(c, text_width))
        .sum();
    text_h + children_h
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

    // ─── Unordered list ───

    #[test]
    fn render_unordered_list() {
        let items = vec![
            ListItem { text: Text::from("item 1"), children: vec![] },
            ListItem { text: Text::from("item 2"), children: vec![] },
        ];
        let area = Rect::new(0, 0, 20, 10);
        let mut buf = Buffer::empty(area);
        render(&items, 1, false, area, &mut buf, &TestTheme);
        let got: String = buf.content.iter().map(|c| c.symbol().chars().next().unwrap_or(' ')).collect();
        assert!(got.contains("item 1"));
        assert!(got.contains("item 2"));
        assert!(got.contains('•'), "Should have bullet");
    }

    #[test]
    fn render_unordered_empty_items() {
        let items: Vec<ListItem> = vec![];
        let area = Rect::new(0, 0, 20, 10);
        let mut buf = Buffer::empty(area);
        render(&items, 1, false, area, &mut buf, &TestTheme);
        // Should not panic
    }

    // ─── Ordered list ───

    #[test]
    fn render_ordered_list() {
        let items = vec![
            ListItem { text: Text::from("first"), children: vec![] },
            ListItem { text: Text::from("second"), children: vec![] },
        ];
        let area = Rect::new(0, 0, 20, 10);
        let mut buf = Buffer::empty(area);
        render(&items, 1, true, area, &mut buf, &TestTheme);
        let got: String = buf.content.iter().map(|c| c.symbol().chars().next().unwrap_or(' ')).collect();
        assert!(got.contains("1."));
        assert!(got.contains("2."));
    }

    #[test]
    fn render_ordered_list_custom_start() {
        let items = vec![
            ListItem { text: Text::from("three"), children: vec![] },
            ListItem { text: Text::from("four"), children: vec![] },
        ];
        let area = Rect::new(0, 0, 20, 10);
        let mut buf = Buffer::empty(area);
        render(&items, 3, true, area, &mut buf, &TestTheme);
        let got: String = buf.content.iter().map(|c| c.symbol().chars().next().unwrap_or(' ')).collect();
        assert!(got.contains("3."));
        assert!(got.contains("4."));
    }

    // ─── Nested list ───

    #[test]
    fn render_nested_list() {
        let items = vec![
            ListItem {
                text: Text::from("parent"),
                children: vec![
                    ListItem { text: Text::from("child A"), children: vec![] },
                    ListItem { text: Text::from("child B"), children: vec![] },
                ],
            },
        ];
        let area = Rect::new(0, 0, 30, 10);
        let mut buf = Buffer::empty(area);
        render(&items, 1, false, area, &mut buf, &TestTheme);
        let got: String = buf.content.iter().map(|c| c.symbol().chars().next().unwrap_or(' ')).collect();
        assert!(got.contains("parent"));
        assert!(got.contains("child A"));
        assert!(got.contains("child B"));
    }

    #[test]
    fn render_deeply_nested_list() {
        let items = vec![
            ListItem {
                text: Text::from("level 1"),
                children: vec![
                    ListItem {
                        text: Text::from("level 2"),
                        children: vec![
                            ListItem { text: Text::from("level 3"), children: vec![] },
                        ],
                    },
                ],
            },
        ];
        let area = Rect::new(0, 0, 40, 10);
        let mut buf = Buffer::empty(area);
        render(&items, 1, false, area, &mut buf, &TestTheme);
        let got: String = buf.content.iter().map(|c| c.symbol().chars().next().unwrap_or(' ')).collect();
        assert!(got.contains("level 1"));
        assert!(got.contains("level 2"));
        assert!(got.contains("level 3"));
    }

    // ─── Edge cases ───

    #[test]
    fn render_zero_area() {
        let items = vec![ListItem { text: Text::from("item"), children: vec![] }];
        let area = Rect::new(0, 0, 0, 0);
        let mut buf = Buffer::empty(area);
        render(&items, 1, false, area, &mut buf, &TestTheme);
        // Should not panic
    }

    #[test]
    fn render_item_with_empty_text() {
        let items = vec![
            ListItem { text: Text::from(vec![]), children: vec![] },
        ];
        let area = Rect::new(0, 0, 20, 5);
        let mut buf = Buffer::empty(area);
        render(&items, 1, false, area, &mut buf, &TestTheme);
        // Should render just the bullet
        let got: String = buf.content.iter().map(|c| c.symbol().chars().next().unwrap_or(' ')).collect();
        assert!(got.contains('•'));
    }

    #[test]
    fn render_clipped_area() {
        let items = vec![
            ListItem { text: Text::from("item 1"), children: vec![] },
            ListItem { text: Text::from("item 2"), children: vec![] },
            ListItem { text: Text::from("item 3"), children: vec![] },
        ];
        let area = Rect::new(0, 0, 20, 2); // Only room for 1 item
        let mut buf = Buffer::empty(area);
        render(&items, 1, false, area, &mut buf, &TestTheme);
        // Should not panic — clips at area boundary
    }

    // ─── list_item_text_height ───

    #[test]
    fn list_item_text_height_empty() {
        let item = ListItem { text: Text::from(vec![]), children: vec![] };
        assert_eq!(list_item_text_height(&item, 40), 1);
    }

    #[test]
    fn list_item_text_height_single_line() {
        let item = ListItem { text: Text::from("short"), children: vec![] };
        assert_eq!(list_item_text_height(&item, 40), 2); // text + blank
    }

    #[test]
    fn list_item_text_height_wraps() {
        let long = "a".repeat(80);
        let item = ListItem { text: Text::from(long), children: vec![] };
        let h = list_item_text_height(&item, 40);
        assert!(h >= 2, "Long text should wrap: height={}", h);
    }

    #[test]
    fn list_item_text_height_zero_width() {
        let item = ListItem { text: Text::from("text"), children: vec![] };
        assert_eq!(list_item_text_height(&item, 0), 1);
    }
}
