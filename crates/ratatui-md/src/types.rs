//! Structured markdown content blocks.
//!
//! Each block implements `HeightAware` (reports height for a given width).
//! Rendering is done via `render_block()` which takes a `Buffer`.

use crate::theme::MdTheme;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;

// ─── Public types ───

/// Trait for types that can report their rendered height given a width.
pub trait HeightAware {
    fn height(&self, width: usize, theme: &dyn MdTheme) -> usize;
}

/// A structured markdown content block.
#[derive(Debug, Clone)]
pub enum ContentBlock {
    Paragraph { text: ratatui::text::Text<'static> },
    Heading { level: u8, text: ratatui::text::Text<'static> },
    CodeBlock { lang: String, content: String },
    Table { headers: Vec<String>, rows: Vec<Vec<String>> },
    List { ordered: bool, start: u32, items: Vec<ListItem> },
    TaskList { items: Vec<TaskListItem> },
    BlockQuote { text: ratatui::text::Text<'static> },
    Rule,
}

/// A list item with optional nested children.
#[derive(Debug, Clone)]
pub struct ListItem {
    pub text: ratatui::text::Text<'static>,
    pub children: Vec<Self>,
}

/// A task list item with checkbox state.
#[derive(Debug, Clone)]
pub struct TaskListItem {
    pub checked: bool,
    pub text: ratatui::text::Text<'static>,
}

// ─── Re-export ratatui types used by tests and consumers ───

pub use ratatui::text::{Line, Span, Text};

// ─── Re-export height helpers used by tests and consumers ───

pub use crate::height::{compute_col_widths, wrapped_height, wrapped_line_count};

// ─── HeightAware impl ───

impl HeightAware for ContentBlock {
    fn height(&self, width: usize, theme: &dyn MdTheme) -> usize {
        crate::height::content_block_height(self, width, theme)
    }
}

// ─── Rendering ───

/// Render a single ContentBlock into a Rect area using a Buffer.
/// Called by the chat renderer for each block in an AssistantMessage.
pub fn render_block(block: &ContentBlock, area: Rect, buf: &mut Buffer, theme: &dyn MdTheme) {
    if area.width == 0 || area.height == 0 {
        return;
    }

    match block {
        ContentBlock::Paragraph { text } => crate::paragraph::render(text, area, buf, theme),
        ContentBlock::Heading { text, .. } => crate::heading::render(text, area, buf, theme),
        ContentBlock::CodeBlock { lang, content } => crate::code::render(lang, content, area, buf, theme),
        ContentBlock::Table { headers, rows } => crate::table::render(headers, rows, area, buf, theme),
        ContentBlock::List { ordered, start, items } => {
            crate::list::render(items, *start, *ordered, area, buf, theme)
        }
        ContentBlock::TaskList { items } => crate::task_list::render(items, area, buf, theme),
        ContentBlock::BlockQuote { text } => crate::blockquote::render(text, area, buf, theme),
        ContentBlock::Rule => crate::rule::render(area, buf, theme),
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
    fn render_block_zero_width() {
        let block = ContentBlock::Paragraph { text: Text::from("hello") };
        let area = Rect::new(0, 0, 0, 2);
        let mut buf = Buffer::empty(area);
        render_block(&block, area, &mut buf, &TestTheme);
        // Should return early without panicking
    }

    #[test]
    fn render_block_zero_height() {
        let block = ContentBlock::Paragraph { text: Text::from("hello") };
        let area = Rect::new(0, 0, 10, 0);
        let mut buf = Buffer::empty(area);
        render_block(&block, area, &mut buf, &TestTheme);
    }

    #[test]
    fn render_block_paragraph() {
        let block = ContentBlock::Paragraph { text: Text::from("hello") };
        let area = Rect::new(0, 0, 10, 2);
        let mut buf = Buffer::empty(area);
        render_block(&block, area, &mut buf, &TestTheme);
        let got: String = buf.content.iter().map(|c| c.symbol().chars().next().unwrap_or(' ')).collect();
        assert!(got.starts_with("hello"));
    }

    #[test]
    fn render_block_heading() {
        let block = ContentBlock::Heading { level: 1, text: Text::from("Title") };
        let area = Rect::new(0, 0, 10, 2);
        let mut buf = Buffer::empty(area);
        render_block(&block, area, &mut buf, &TestTheme);
        let got: String = buf.content.iter().map(|c| c.symbol().chars().next().unwrap_or(' ')).collect();
        assert!(got.starts_with("Title"));
    }

    #[test]
    fn render_block_code() {
        let block = ContentBlock::CodeBlock { lang: "rust".into(), content: "code".into() };
        let area = Rect::new(0, 0, 20, 5);
        let mut buf = Buffer::empty(area);
        render_block(&block, area, &mut buf, &TestTheme);
        let got: String = buf.content.iter().map(|c| c.symbol().chars().next().unwrap_or(' ')).collect();
        assert!(got.contains("rust"));
    }

    #[test]
    fn render_block_table() {
        let block = ContentBlock::Table {
            headers: vec!["A".into()],
            rows: vec![vec!["1".into()]],
        };
        let area = Rect::new(0, 0, 15, 10);
        let mut buf = Buffer::empty(area);
        render_block(&block, area, &mut buf, &TestTheme);
        let got: String = buf.content.iter().map(|c| c.symbol().chars().next().unwrap_or(' ')).collect();
        assert!(got.contains('┌'));
    }

    #[test]
    fn render_block_list() {
        let block = ContentBlock::List {
            ordered: false, start: 1,
            items: vec![ListItem { text: Text::from("item"), children: vec![] }],
        };
        let area = Rect::new(0, 0, 20, 5);
        let mut buf = Buffer::empty(area);
        render_block(&block, area, &mut buf, &TestTheme);
        let got: String = buf.content.iter().map(|c| c.symbol().chars().next().unwrap_or(' ')).collect();
        assert!(got.contains('•'));
    }

    #[test]
    fn render_block_task_list() {
        let block = ContentBlock::TaskList {
            items: vec![TaskListItem { checked: true, text: Text::from("done") }],
        };
        let area = Rect::new(0, 0, 20, 5);
        let mut buf = Buffer::empty(area);
        render_block(&block, area, &mut buf, &TestTheme);
        let got: String = buf.content.iter().map(|c| c.symbol().chars().next().unwrap_or(' ')).collect();
        assert!(got.contains("☑"));
    }

    #[test]
    fn render_block_blockquote() {
        let block = ContentBlock::BlockQuote { text: Text::from("quote") };
        let area = Rect::new(0, 0, 15, 3);
        let mut buf = Buffer::empty(area);
        render_block(&block, area, &mut buf, &TestTheme);
        let got: String = buf.content.iter().map(|c| c.symbol().chars().next().unwrap_or(' ')).collect();
        assert!(got.contains("quote"));
    }

    #[test]
    fn render_block_rule() {
        let block = ContentBlock::Rule;
        let area = Rect::new(0, 0, 10, 1);
        let mut buf = Buffer::empty(area);
        render_block(&block, area, &mut buf, &TestTheme);
        // "─" is wide (2 cells), width=10 → 5 chars → 10 cells filled
        assert_eq!(buf.content.len(), 10);
        let dash_cells: usize = buf.content.iter().filter(|c| c.symbol() == "─").count();
        assert_eq!(dash_cells, 5);
    }

    #[test]
    fn height_aware_paragraph() {
        let block: ContentBlock = ContentBlock::Paragraph { text: Text::from("hello") };
        let h = block.height(80, &TestTheme);
        assert_eq!(h, 2);
    }

    #[test]
    fn height_aware_rule() {
        let block: ContentBlock = ContentBlock::Rule;
        assert_eq!(block.height(80, &TestTheme), 2);
    }

    #[test]
    fn list_item_clone() {
        let item = ListItem { text: Text::from("test"), children: vec![] };
        let cloned = item.clone();
        assert_eq!(
            cloned.text.lines[0].spans[0].content.as_ref(),
            "test"
        );
    }

    #[test]
    fn task_list_item_clone() {
        let item = TaskListItem { checked: true, text: Text::from("test") };
        let cloned = item.clone();
        assert!(cloned.checked);
    }
}
