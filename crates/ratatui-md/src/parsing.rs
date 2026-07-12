//! Markdown parser that converts pulldown-cmark events to structured `ContentBlock`s.

use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span, Text};
use pulldown_cmark::{Event, Options, Parser, Tag, TagEnd, CodeBlockKind};

use crate::theme::MdTheme;
use crate::types::{ContentBlock, ListItem, TaskListItem};

/// State for tracking list nesting during parsing.
struct ListContext {
    _ordered: bool,
    start: u32,
    items: Vec<ListItem>,
}

/// Markdown renderer that converts pulldown-cmark events to structured content blocks.
pub struct MarkdownRenderer<'a> {
    pub theme: &'a dyn MdTheme,
}

impl<'a> MarkdownRenderer<'a> {
    pub fn new(theme: &'a dyn MdTheme) -> Self {
        Self { theme }
    }

    /// Parse markdown text and return structured content blocks.
    pub fn render(&self, markdown: &str) -> Vec<ContentBlock> {
        let mut options = Options::empty();
        options.insert(Options::ENABLE_STRIKETHROUGH);
        options.insert(Options::ENABLE_TABLES);
        options.insert(Options::ENABLE_FOOTNOTES);
        options.insert(Options::ENABLE_TASKLISTS);

        // Normalize Unicode bullets to ASCII so pulldown-cmark recognizes lists.
        let normalized = Self::normalize_list_markers(markdown);
        let parser = Parser::new_ext(&normalized, options);
        self.events_to_blocks(parser)
    }

    /// Normalize Unicode list markers to ASCII so pulldown-cmark recognizes them.
    pub fn normalize_list_markers(md: &str) -> String {
        md.lines()
            .map(|line| {
                let trimmed = line.trim_start();
                let leading_ws = &line[..line.len() - trimmed.len()];
                if trimmed.starts_with('•') || trimmed.starts_with('◦')
                    || trimmed.starts_with('▪') || trimmed.starts_with('■')
                    || trimmed.starts_with('●') || trimmed.starts_with('○')
                    || trimmed.starts_with('◇') || trimmed.starts_with('▸')
                {
                    let rest = trimmed.chars().skip(1).collect::<String>();
                    let rest = rest.strip_prefix(' ').unwrap_or(&rest);
                    format!("{}- {}", leading_ws, rest)
                } else {
                    line.to_string()
                }
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn events_to_blocks(&self, parser: Parser) -> Vec<ContentBlock> {
        let mut blocks: Vec<ContentBlock> = Vec::new();

        // Inline state
        let mut current_spans: Vec<Span<'static>> = Vec::new();
        let mut current_lines: Vec<Line<'static>> = Vec::new();
        let mut in_bold = false;
        let mut in_italic = false;
        let mut in_strikethrough = false;
        let mut is_heading = false;
        let mut heading_level: u8 = 0;
        let mut in_link = false;
        let mut link_url: String = String::new();
        let mut link_text: String = String::new();
        let mut in_block_quote = false;

        // Block-level state
        let mut in_code_block = false;
        let mut code_block_lang: String = String::new();
        let mut code_block_content: String = String::new();

        // Table state
        let mut in_table = false;
        let mut _table_col_count: usize = 0;
        let mut table_headers: Vec<String> = Vec::new();
        let mut table_rows: Vec<Vec<String>> = Vec::new();
        let mut table_current_row: Vec<String> = Vec::new();
        let mut table_current_cell: String = String::new();
        let mut in_table_cell = false;

        // List state — stack for nesting
        let mut list_stack: Vec<ListContext> = Vec::new();
        let mut in_list = false;

        // Task list state
        let mut in_task_list = false;
        let mut task_items: Vec<TaskListItem> = Vec::new();
        let mut task_checked: bool = false;

        // Helper: flush current spans to current_lines
        let flush_spans = |spans: &mut Vec<Span<'static>>, lines: &mut Vec<Line<'static>>| {
            if !spans.is_empty() {
                lines.push(Line::from(std::mem::take(spans)));
            }
        };

        // Helper: flush current lines to a paragraph block
        let flush_paragraph = |blocks: &mut Vec<ContentBlock>,
                                   lines: &mut Vec<Line<'static>>,
                                   heading: bool,
                                   level: u8,
                                   _theme: &dyn MdTheme| {
            if lines.is_empty() {
                return;
            }
            let text = Text::from(std::mem::take(lines));
            if heading {
                blocks.push(ContentBlock::Heading { level, text });
            } else {
                blocks.push(ContentBlock::Paragraph { text });
            }
        };

        // Helper: get span style for inline formatting
        let get_span_style = |bold: bool, italic: bool, strikethrough: bool, heading: bool, theme: &dyn MdTheme| -> Style {
            let mut style = if heading {
                theme.heading_style()
            } else {
                theme.text_style()
            };
            if bold { style = style.add_modifier(Modifier::BOLD); }
            if italic { style = style.add_modifier(Modifier::ITALIC); }
            if strikethrough {
                style = style.add_modifier(Modifier::CROSSED_OUT);
                style = style.fg(theme.text_muted_color());
            }
            style
        };

        for event in parser {
            match event {
                // ─── Headings ───
                Event::Start(Tag::Heading { level, .. }) => {
                    flush_spans(&mut current_spans, &mut current_lines);
                    is_heading = true;
                    heading_level = level as u8;
                }
                Event::End(TagEnd::Heading(_)) => {
                    flush_spans(&mut current_spans, &mut current_lines);
                    flush_paragraph(&mut blocks, &mut current_lines, true, heading_level, self.theme);
                    is_heading = false;
                }

                // ─── Code blocks ───
                Event::Start(Tag::CodeBlock(kind)) => {
                    flush_spans(&mut current_spans, &mut current_lines);
                    flush_paragraph(&mut blocks, &mut current_lines, false, 0, self.theme);
                    in_code_block = true;
                    code_block_content.clear();
                    code_block_lang = match kind {
                        CodeBlockKind::Fenced(lang) => lang.to_string(),
                        CodeBlockKind::Indented => String::new(),
                    };
                }
                Event::End(TagEnd::CodeBlock) => {
                    in_code_block = false;
                    blocks.push(ContentBlock::CodeBlock {
                        lang: std::mem::take(&mut code_block_lang),
                        content: std::mem::take(&mut code_block_content),
                    });
                }

                // ─── Lists ───
                Event::Start(Tag::List(start)) => {
                    flush_spans(&mut current_spans, &mut current_lines);
                    if in_list {
                        // Nested list: flush current text into the parent item's text
                        // before pushing the new list context
                        if let Some(last_item) = list_stack.last_mut().and_then(|p| p.items.last_mut()) {
                            last_item.text = Text::from(std::mem::take(&mut current_lines));
                        }
                    } else {
                        flush_paragraph(&mut blocks, &mut current_lines, false, 0, self.theme);
                    }
                    let ordered = start.is_some();
                    let start_val = start.map(|s| s as u32).unwrap_or(1);
                    list_stack.push(ListContext {
                        _ordered: ordered,
                        start: start_val,
                        items: Vec::new(),
                    });
                    in_list = true;
                }
                Event::End(TagEnd::List(is_ordered)) => {
                    if let Some(ctx) = list_stack.pop() {
                        // Filter out empty list items (artifacts of LLM formatting noise)
                        let items: Vec<_> = ctx.items.into_iter()
                            .filter(|item| !item.text.lines.is_empty() || !item.children.is_empty())
                            .collect();
                        if list_stack.is_empty() {
                            // Top-level list ended — push as block
                            if items.is_empty() {
                                in_list = false;
                                continue;
                            }
                            if is_ordered {
                                blocks.push(ContentBlock::List {
                                    ordered: true,
                                    start: ctx.start,
                                    items,
                                });
                            } else {
                                blocks.push(ContentBlock::List {
                                    ordered: false,
                                    start: 1,
                                    items,
                                });
                            }
                            in_list = false;
                        } else if let Some(last_item) = list_stack.last_mut().and_then(|p| p.items.last_mut()) {
                            // Nested list ended — attach children to parent's last item
                            if !items.is_empty() {
                                last_item.children.extend(items);
                            }
                        }
                    }
                }
                Event::Start(Tag::Item) => {
                    flush_spans(&mut current_spans, &mut current_lines);
                    // Start a new list item with empty text
                    if let Some(ctx) = list_stack.last_mut() {
                        ctx.items.push(ListItem {
                            text: Text::from(Vec::new()),
                            children: Vec::new(),
                        });
                    }
                }
                Event::End(TagEnd::Item) => {
                    // Flush accumulated text into the current item.
                    // Use extend instead of replace: if the item already has text
                    // (e.g. captured before a nested list started), we append rather
                    // than overwrite with empty current_lines.
                    if let Some(last_item) = list_stack.last_mut().and_then(|ctx| ctx.items.last_mut()) {
                        flush_spans(&mut current_spans, &mut current_lines);
                        if !current_lines.is_empty() {
                            last_item.text.lines.extend(std::mem::take(&mut current_lines));
                        }
                    }
                }

                // ─── Task list markers ───
                Event::TaskListMarker(checked) => {
                    task_checked = checked;
                    if !in_task_list {
                        // Switch to task list mode
                        flush_spans(&mut current_spans, &mut current_lines);
                        flush_paragraph(&mut blocks, &mut current_lines, false, 0, self.theme);
                        in_task_list = true;
                        task_items.clear();
                    }
                }

                // ─── Inline formatting ───
                Event::Start(Tag::Emphasis) => { in_italic = true; }
                Event::End(TagEnd::Emphasis) => { in_italic = false; }
                Event::Start(Tag::Strong) => { in_bold = true; }
                Event::End(TagEnd::Strong) => { in_bold = false; }
                Event::Start(Tag::Strikethrough) => { in_strikethrough = true; }
                Event::End(TagEnd::Strikethrough) => { in_strikethrough = false; }

                Event::Start(Tag::Link { dest_url: url, .. }) => {
                    in_link = true;
                    link_url = url.to_string();
                    link_text.clear();
                }
                Event::End(TagEnd::Link) => {
                    if in_link {
                        in_link = false;
                        let text = if link_text.is_empty() { &link_url } else { &link_text };
                        current_spans.push(Span::styled(
                            format!("{} ({})", text, link_url),
                            self.theme.link_style(),
                        ));
                        link_text.clear();
                        link_url.clear();
                    }
                }

                // ─── Block quotes ───
                Event::Start(Tag::BlockQuote(_)) => {
                    flush_spans(&mut current_spans, &mut current_lines);
                    flush_paragraph(&mut blocks, &mut current_lines, false, 0, self.theme);
                    in_block_quote = true;
                }
                Event::End(TagEnd::BlockQuote(_)) => {
                    flush_spans(&mut current_spans, &mut current_lines);
                    if !current_lines.is_empty() {
                        let text = Text::from(std::mem::take(&mut current_lines));
                        blocks.push(ContentBlock::BlockQuote { text });
                    }
                    in_block_quote = false;
                }

                // ─── Tables ───
                Event::Start(Tag::Table(_alignments)) => {
                    flush_spans(&mut current_spans, &mut current_lines);
                    flush_paragraph(&mut blocks, &mut current_lines, false, 0, self.theme);
                    in_table = true;
                    table_headers.clear();
                    table_rows.clear();
                    table_current_row.clear();
                    table_current_cell.clear();
                }
                Event::End(TagEnd::Table) => {
                    // Push any remaining row
                    if !table_current_row.is_empty() {
                        table_rows.push(std::mem::take(&mut table_current_row));
                    }
                    blocks.push(ContentBlock::Table {
                        headers: std::mem::take(&mut table_headers),
                        rows: std::mem::take(&mut table_rows),
                    });
                    in_table = false;
                    _table_col_count = 0;
                }
                Event::End(TagEnd::TableHead) => {
                    table_headers = std::mem::take(&mut table_current_row);
                }
                Event::End(TagEnd::TableRow) => {
                    if !table_current_row.is_empty() {
                        table_rows.push(std::mem::take(&mut table_current_row));
                    }
                }
                Event::Start(Tag::TableCell) => {
                    let was_in_cell = in_table_cell;
                    in_table_cell = true;
                    if was_in_cell {
                        table_current_cell.clear();
                    }
                }
                Event::End(TagEnd::TableCell) => {
                    in_table_cell = false;
                    table_current_row.push(std::mem::take(&mut table_current_cell));
                }

                // ─── Paragraphs ───
                Event::Start(Tag::Paragraph) => {}
                Event::End(TagEnd::Paragraph) => {
                    if in_list {
                        // Inside a list item — text goes to the item, not a separate block
                        // (flushed on End(Item))
                    } else if in_task_list {
                        // Task list item text
                        flush_spans(&mut current_spans, &mut current_lines);
                        let text = Text::from(std::mem::take(&mut current_lines));
                        task_items.push(TaskListItem {
                            checked: task_checked,
                            text,
                        });
                    } else if in_block_quote {
                        // Inside a block quote — text stays in current_lines
                        // for End(BlockQuote) to collect into a BlockQuote block
                    } else {
                        flush_spans(&mut current_spans, &mut current_lines);
                        flush_paragraph(&mut blocks, &mut current_lines, false, 0, self.theme);
                    }
                }

                // ─── Text and Code ───
                Event::Text(text) => {
                    if in_code_block {
                        code_block_content.push_str(&text);
                        continue;
                    }
                    if in_table {
                        // In table: go to current cell
                        if in_table_cell {
                            table_current_cell.push_str(&text);
                        }
                        continue;
                    }
                    if in_link {
                        link_text.push_str(&text);
                        continue;
                    }
                    let style = get_span_style(in_bold, in_italic, in_strikethrough, is_heading, self.theme);
                    current_spans.push(Span::styled(text.to_string(), style));
                }
                Event::Code(text) => {
                    if in_code_block {
                        code_block_content.push_str(&text);
                        continue;
                    }
                    if in_table && in_table_cell {
                        table_current_cell.push_str(&text);
                        continue;
                    }
                    current_spans.push(Span::styled(text.to_string(), self.theme.inline_code_style()));
                }

                // ─── Breaks ───
                Event::SoftBreak => {
                    if !in_code_block {
                        flush_spans(&mut current_spans, &mut current_lines);
                    }
                }
                Event::HardBreak => {
                    if !in_code_block {
                        flush_spans(&mut current_spans, &mut current_lines);
                        current_lines.push(Line::default());
                    }
                }

                // ─── Horizontal rule ───
                Event::Rule => {
                    flush_spans(&mut current_spans, &mut current_lines);
                    flush_paragraph(&mut blocks, &mut current_lines, false, 0, self.theme);
                    blocks.push(ContentBlock::Rule);
                }

                // ─── Task list end ───
                _ => {}
            }
        }

        // Flush any remaining state
        flush_spans(&mut current_spans, &mut current_lines);
        if !current_lines.is_empty() && !in_list && !in_task_list {
            flush_paragraph(&mut blocks, &mut current_lines, false, 0, self.theme);
        }
        if in_task_list && !task_items.is_empty() {
            blocks.push(ContentBlock::TaskList {
                items: std::mem::take(&mut task_items),
            });
        }

        blocks
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::style::{Color, Style, Modifier};

    struct TestTheme;
    impl MdTheme for TestTheme {
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

    fn render(md: &str) -> Vec<ContentBlock> {
        MarkdownRenderer::new(&TestTheme).render(md)
    }

    fn block_type(b: &ContentBlock) -> &'static str {
        match b {
            ContentBlock::Paragraph { .. } => "Paragraph",
            ContentBlock::Heading { .. } => "Heading",
            ContentBlock::CodeBlock { .. } => "CodeBlock",
            ContentBlock::Table { .. } => "Table",
            ContentBlock::List { .. } => "List",
            ContentBlock::TaskList { .. } => "TaskList",
            ContentBlock::BlockQuote { .. } => "BlockQuote",
            ContentBlock::Rule => "Rule",
        }
    }

    fn text_str(text: &ratatui::text::Text<'_>) -> String {
        let mut result = String::new();
        for line in &text.lines {
            for span in &line.spans {
                result.push_str(span.content.as_ref());
            }
        }
        result
    }

    // ─── Empty / whitespace ───

    #[test]
    fn render_empty_string() {
        assert!(render("").is_empty());
    }

    #[test]
    fn render_whitespace_only() {
        assert!(render("   \n\n   ").is_empty());
    }

    // ─── Paragraphs ───

    #[test]
    fn render_single_paragraph() {
        let blocks = render("Hello world");
        assert_eq!(blocks.len(), 1);
        assert_eq!(block_type(&blocks[0]), "Paragraph");
        if let ContentBlock::Paragraph { text } = &blocks[0] {
            assert_eq!(text_str(text), "Hello world");
        }
    }

    #[test]
    fn render_multiple_paragraphs() {
        let blocks = render("Para one\n\nPara two");
        assert_eq!(blocks.len(), 2);
        assert_eq!(block_type(&blocks[0]), "Paragraph");
        assert_eq!(block_type(&blocks[1]), "Paragraph");
    }

    // ─── Headings ───

    #[test]
    fn render_h1() {
        let blocks = render("# Title");
        assert_eq!(blocks.len(), 1);
        if let ContentBlock::Heading { level, text } = &blocks[0] {
            assert_eq!(*level, 1);
            assert_eq!(text_str(text), "Title");
        } else {
            panic!("Expected Heading");
        }
    }

    #[test]
    fn render_h2() {
        let blocks = render("## Subtitle");
        assert_eq!(blocks.len(), 1);
        if let ContentBlock::Heading { level, text } = &blocks[0] {
            assert_eq!(*level, 2);
            assert_eq!(text_str(text), "Subtitle");
        } else {
            panic!("Expected Heading");
        }
    }

    #[test]
    fn render_h3_through_h6() {
        for (level, prefix) in [(3, "###"), (4, "####"), (5, "#####"), (6, "######")] {
            let blocks = render(&format!("{} Heading {}", prefix, level));
            assert_eq!(blocks.len(), 1, "H{} should produce 1 block", level);
            if let ContentBlock::Heading { level: l, .. } = &blocks[0] {
                assert_eq!(*l, level, "H{} level mismatch", level);
            } else {
                panic!("Expected Heading for H{}", level);
            }
        }
    }

    // ─── Code blocks ───

    #[test]
    fn render_fenced_code_with_language() {
        let blocks = render("```rust\nfn main() {}\n```");
        assert_eq!(blocks.len(), 1);
        if let ContentBlock::CodeBlock { lang, content } = &blocks[0] {
            assert_eq!(lang, "rust");
            assert_eq!(content, "fn main() {}\n");
        } else {
            panic!("Expected CodeBlock");
        }
    }

    #[test]
    fn render_indented_code_block() {
        let blocks = render("    fn main() {}\n");
        assert_eq!(blocks.len(), 1);
        if let ContentBlock::CodeBlock { lang, content } = &blocks[0] {
            assert!(lang.is_empty(), "Indented code should have empty lang");
            assert!(content.contains("fn main"));
        } else {
            panic!("Expected CodeBlock");
        }
    }

    #[test]
    fn render_empty_code_block() {
        let blocks = render("```\n```\n");
        assert_eq!(blocks.len(), 1);
        if let ContentBlock::CodeBlock { lang, content } = &blocks[0] {
            assert!(lang.is_empty());
            assert!(content.is_empty());
        } else {
            panic!("Expected CodeBlock");
        }
    }

    #[test]
    fn render_code_block_multiline() {
        let blocks = render("```python\ndef foo():\n    return 42\n```\n");
        if let ContentBlock::CodeBlock { lang, content } = &blocks[0] {
            assert_eq!(lang, "python");
            assert!(content.contains("def foo"));
            assert!(content.contains("return 42"));
        } else {
            panic!("Expected CodeBlock");
        }
    }

    // ─── Inline formatting ───

    #[test]
    fn render_bold_text() {
        let blocks = render("**bold text**");
        if let ContentBlock::Paragraph { text } = &blocks[0] {
            assert!(!text.lines.is_empty());
            let spans = &text.lines[0].spans;
            assert!(!spans.is_empty());
            assert!(spans[0].style.add_modifier(Modifier::BOLD) == spans[0].style, "Should have BOLD modifier");
        } else {
            panic!("Expected Paragraph");
        }
    }

    #[test]
    fn render_italic_text() {
        let blocks = render("*italic text*");
        if let ContentBlock::Paragraph { text } = &blocks[0] {
            assert!(!text.lines.is_empty());
            let spans = &text.lines[0].spans;
            assert!(!spans.is_empty());
            assert!(spans[0].style.add_modifier(Modifier::ITALIC) == spans[0].style, "Should have ITALIC modifier");
        } else {
            panic!("Expected Paragraph");
        }
    }

    #[test]
    fn render_strikethrough_text() {
        let blocks = render("~~deleted~~");
        if let ContentBlock::Paragraph { text } = &blocks[0] {
            let spans = &text.lines[0].spans;
            assert!(!spans.is_empty());
            assert!(spans[0].style.add_modifier(Modifier::CROSSED_OUT) == spans[0].style, "Should have CROSSED_OUT");
            assert_eq!(spans[0].style.fg, Some(Color::Gray));
        } else {
            panic!("Expected Paragraph");
        }
    }

    #[test]
    fn render_bold_italic_combined() {
        let blocks = render("***bold italic***");
        if let ContentBlock::Paragraph { text } = &blocks[0] {
            let spans = &text.lines[0].spans;
            assert!(!spans.is_empty());
            let s = spans[0].style;
            assert!(s.add_modifier(Modifier::BOLD) == s, "Should have BOLD");
            assert!(s.add_modifier(Modifier::ITALIC) == s, "Should have ITALIC");
        } else {
            panic!("Expected Paragraph");
        }
    }

    #[test]
    fn render_inline_code() {
        let blocks = render("Use `cargo test`");
        if let ContentBlock::Paragraph { text } = &blocks[0] {
            let spans = &text.lines[0].spans;
            // Should have at least 2 spans: "Use " and "cargo test"
            assert!(spans.len() >= 2, "Expected at least 2 spans, got {}", spans.len());
        } else {
            panic!("Expected Paragraph");
        }
    }

    // ─── Links ───

    #[test]
    fn render_link_with_text() {
        let blocks = render("[click here](https://example.com)");
        if let ContentBlock::Paragraph { text } = &blocks[0] {
            let t = text_str(text);
            assert!(t.contains("click here"));
            assert!(t.contains("https://example.com"));
        } else {
            panic!("Expected Paragraph");
        }
    }

    #[test]
    fn render_link_without_text() {
        let blocks = render("[](https://example.com)");
        if let ContentBlock::Paragraph { text } = &blocks[0] {
            let t = text_str(text);
            // When link_text is empty, uses URL as text
            assert!(t.contains("https://example.com"));
        } else {
            panic!("Expected Paragraph");
        }
    }

    // ─── Block quotes ───

    #[test]
    fn render_blockquote() {
        let blocks = render("> This is a quote\n");
        assert_eq!(blocks.len(), 1);
        assert_eq!(block_type(&blocks[0]), "BlockQuote");
        if let ContentBlock::BlockQuote { text } = &blocks[0] {
            assert_eq!(text_str(text), "This is a quote");
        }
    }

    #[test]
    fn render_blockquote_multiline() {
        let blocks = render("> Line one\n> Line two\n");
        assert_eq!(blocks.len(), 1);
        assert_eq!(block_type(&blocks[0]), "BlockQuote");
    }

    #[test]
    fn render_empty_blockquote() {
        let blocks = render(">");
        assert!(blocks.is_empty(), "Empty blockquote should produce no blocks");
    }

    // ─── Horizontal rule ───

    #[test]
    fn render_rule() {
        let blocks = render("---");
        assert_eq!(blocks.len(), 1);
        assert_eq!(block_type(&blocks[0]), "Rule");
    }

    #[test]
    fn render_rule_with_asterisks() {
        let blocks = render("***");
        assert_eq!(blocks.len(), 1);
        assert_eq!(block_type(&blocks[0]), "Rule");
    }

    #[test]
    fn render_rule_with_dashes_long() {
        let blocks = render("-----");
        assert_eq!(blocks.len(), 1);
        assert_eq!(block_type(&blocks[0]), "Rule");
    }

    // ─── Mixed document ───

    #[test]
    fn render_mixed_document() {
        let md = "# Title\n\nSome text\n\n```rust\ncode\n```\n\n- item\n\n> quote\n\n---\n";
        let blocks = render(md);
        let types: Vec<_> = blocks.iter().map(block_type).collect();
        assert!(types.contains(&"Heading"));
        assert!(types.contains(&"Paragraph"));
        assert!(types.contains(&"CodeBlock"));
        assert!(types.contains(&"List"));
        assert!(types.contains(&"BlockQuote"));
        assert!(types.contains(&"Rule"));
    }

    // ─── Ordered list with custom start ───

    #[test]
    fn render_ordered_list_custom_start() {
        let blocks = render("3. Three\n4. Four\n5. Five");
        if let Some(ContentBlock::List { ordered, start, items }) = blocks.iter().find(|b| matches!(b, ContentBlock::List { .. })) {
            assert!(*ordered);
            assert_eq!(*start, 3);
            assert_eq!(items.len(), 3);
        } else {
            panic!("Expected ordered list");
        }
    }

    // ─── Hard break ───

    #[test]
    fn render_hard_break() {
        let blocks = render("Line one  \nLine two");
        if let ContentBlock::Paragraph { text } = &blocks[0] {
            // Hard break (two trailing spaces + newline) adds an extra empty line
            assert!(text.lines.len() >= 3, "Hard break should add extra line, got {} lines", text.lines.len());
        } else {
            panic!("Expected Paragraph");
        }
    }

    // ─── normalize_list_markers ───

    #[test]
    fn normalize_preserves_ascii() {
        let input = "- normal item";
        assert_eq!(MarkdownRenderer::normalize_list_markers(input), input);
    }

    #[test]
    fn normalize_preserves_leading_whitespace() {
        let input = "  • indented";
        let out = MarkdownRenderer::normalize_list_markers(input);
        assert_eq!(out, "  - indented");
    }

    #[test]
    fn normalize_all_bullet_types() {
        let bullets = ['•', '◦', '▪', '■', '●', '○', '◇', '▸'];
        for b in &bullets {
            let input = format!("{} item", b);
            let out = MarkdownRenderer::normalize_list_markers(&input);
            assert_eq!(out, "- item", "Bullet {:?} not normalized", b);
        }
    }

    #[test]
    fn normalize_bullet_no_space_after() {
        let input = "•item";
        let out = MarkdownRenderer::normalize_list_markers(input);
        assert_eq!(out, "- item");
    }
}
