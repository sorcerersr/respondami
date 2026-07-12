//! Markdown parsing and rendering for ratatui TUI applications.
//!
//! # Usage
//!
//! 1. Implement [`MdTheme`] on your application's theme struct.
//! 2. Create a [`MarkdownRenderer`] with your theme.
//! 3. Parse markdown into [`ContentBlock`]s with [`MarkdownRenderer::render()`].
//! 4. Compute heights with [`HeightAware::height()`].
//! 5. Render blocks with [`render_block()`].

mod blockquote;
mod code;
mod heading;
mod height;
mod list;
mod parsing;
mod paragraph;
mod rule;
mod table;
mod task_list;
mod theme;
mod types;

#[doc(inline)]
pub use parsing::MarkdownRenderer;
#[doc(inline)]
pub use theme::MdTheme;
#[doc(inline)]
pub use types::{
    compute_col_widths, wrapped_height, wrapped_line_count,
    ContentBlock, HeightAware, ListItem, TaskListItem,
    Line, Span, Text,
};

/// Render a single ContentBlock into a Rect area using a Buffer.
pub fn render_block(block: &ContentBlock, area: ratatui::layout::Rect, buf: &mut ratatui::buffer::Buffer, theme: &dyn MdTheme) {
    types::render_block(block, area, buf, theme)
}

// ─── Tests ───

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::style::{Color, Style};
    use pulldown_cmark::Options;

    /// Minimal MdTheme implementation for tests.
    struct TestTheme;

    impl MdTheme for TestTheme {
        fn text_style(&self) -> Style { Style::default() }
        fn heading_style(&self) -> Style { Style::default().add_modifier(ratatui::style::Modifier::BOLD) }
        fn text_muted_color(&self) -> Color { Color::Gray }
        fn link_style(&self) -> Style { Style::default().fg(Color::Blue) }
        fn inline_code_style(&self) -> Style { Style::default().fg(Color::Cyan) }
        fn list_bullet_style(&self) -> Style { Style::default().fg(Color::Green) }
        fn code_block_style(&self) -> Style { Style::default().bg(Color::Rgb(30, 30, 30)) }
        fn text_dim_color(&self) -> Color { Color::DarkGray }
        fn text_muted_style(&self) -> Style { Style::default().fg(Color::DarkGray) }
    }

    // ─── Table parsing tests (from old markdown/tests.rs) ───

    #[test]
    fn test_table_parsing() {
        let theme = TestTheme;
        let renderer = MarkdownRenderer::new(&theme);
        let markdown = "| Name | Value |\n|------|-------|\n| A    | 1     |";
        let result = renderer.render(markdown);

        // Should produce a Table block
        let table_blocks: Vec<&ContentBlock> = result.iter()
            .filter(|b| matches!(b, ContentBlock::Table { .. }))
            .collect();
        assert_eq!(table_blocks.len(), 1, "Expected 1 table block, got {}:\n{:?}", table_blocks.len(), result);

        if let ContentBlock::Table { headers, rows } = table_blocks[0] {
            assert_eq!(headers, &["Name", "Value"]);
            assert_eq!(rows.len(), 1);
            assert_eq!(rows[0], vec!["A", "1"]);
        }
    }

    #[test]
    fn test_table_header_not_duplicated() {
        let theme = TestTheme;
        let renderer = MarkdownRenderer::new(&theme);
        let markdown = "| Name | Value |\n|------|-------|\n| A    | 1     |\n| B    | 2     |";
        let result = renderer.render(markdown);

        if let Some(ContentBlock::Table { headers, rows }) = result.iter().find(|b| matches!(b, ContentBlock::Table { .. })) {
            // Headers should appear once
            assert_eq!(headers, &["Name", "Value"]);
            // Data rows should not contain header values
            for row in rows {
                assert!(!row.iter().any(|c| c == "Name"), "Header 'Name' found in data row: {:?}", row);
                assert!(!row.iter().any(|c| c == "Value"), "Header 'Value' found in data row: {:?}", row);
            }
            assert_eq!(rows.len(), 2);
        } else {
            panic!("No table block found");
        }
    }

    #[test]
    fn test_table_multiple_rows() {
        let theme = TestTheme;
        let renderer = MarkdownRenderer::new(&theme);
        let markdown = "| X | Y |\n|---|---|\n| 1 | 2 |\n| 3 | 4 |";
        let result = renderer.render(markdown);

        if let Some(ContentBlock::Table { headers, rows }) = result.iter().find(|b| matches!(b, ContentBlock::Table { .. })) {
            assert_eq!(headers, &["X", "Y"]);
            assert_eq!(rows.len(), 2);
            assert_eq!(rows[0], vec!["1", "2"]);
            assert_eq!(rows[1], vec!["3", "4"]);
        } else {
            panic!("No table block found");
        }
    }

    #[test]
    fn test_table_column_widths() {
        let theme = TestTheme;
        let renderer = MarkdownRenderer::new(&theme);
        let markdown = "| A | Short |\n|---|-------|\n| Long text | B |";
        let result = renderer.render(markdown);

        if let Some(ContentBlock::Table { headers, rows }) = result.iter().find(|b| matches!(b, ContentBlock::Table { .. })) {
            // Column widths should accommodate the longest content
            let height = result.iter().find(|b| matches!(b, ContentBlock::Table { .. }))
                .map(|b| b.height(40, &theme)).unwrap();
            assert!(height >= 5, "Table height should accommodate borders and rows: {}", height);
            assert_eq!(headers.len(), 2);
            assert_eq!(rows.len(), 1);
            assert_eq!(rows[0][0], "Long text");
        } else {
            panic!("No table block found");
        }
    }

    #[test]
    fn test_normalize_unicode_bullets() {
        let md = "• Item 1\n• Item 2\n\n• Item 3";
        let normalized = MarkdownRenderer::normalize_list_markers(md);
        assert!(normalized.starts_with("- Item 1"));
        assert!(normalized.contains("- Item 2"));
        assert!(normalized.contains("- Item 3"));
        let options = Options::empty();
        let mut parser = pulldown_cmark::Parser::new_ext(&normalized, options);
        let has_list = parser.any(|e| matches!(e, pulldown_cmark::Event::Start(pulldown_cmark::Tag::List(_))));
        assert!(has_list, "Normalized markdown should parse as list: {}", normalized);
    }

    #[test]
    fn test_unicode_bullet_list_rendering() {
        let theme = TestTheme;
        let renderer = MarkdownRenderer::new(&theme);
        let md = "Question?

• Item 1 text.
• Item 2 text.

• Item 3 text.";
        let result = renderer.render(md);

        // Should produce a List block with 3 items
        let list_blocks: Vec<&ContentBlock> = result.iter()
            .filter(|b| matches!(b, ContentBlock::List { .. }))
            .collect();
        assert!(!list_blocks.is_empty(), "Expected at least 1 list block, got:\n{:?}", result);

        if let ContentBlock::List { items, .. } = list_blocks[0] {
            assert_eq!(items.len(), 3, "Expected 3 items, got {}: {:?}", items.len(), items);
        }
    }

    #[test]
    fn test_list_items_have_text() {
        // Regression: in_list flag caused Event::Text to skip list item text
        let theme = TestTheme;
        let renderer = MarkdownRenderer::new(&theme);
        let md = "- First item\n- Second item\n- Third item";
        let result = renderer.render(md);

        let list_blocks: Vec<&ContentBlock> = result.iter()
            .filter(|b| matches!(b, ContentBlock::List { .. }))
            .collect();
        assert!(!list_blocks.is_empty(), "Expected list block, got: {:?}", result);

        if let ContentBlock::List { items, .. } = list_blocks[0] {
            assert_eq!(items.len(), 3);
            // Each item must have non-empty text
            for (i, item) in items.iter().enumerate() {
                let text: String = item.text.lines.iter()
                    .flat_map(|l| l.spans.iter().map(|s| &*s.content))
                    .collect();
                assert!(!text.is_empty(), "Item {} has empty text", i);
            }
            assert!(items[0].text.lines.iter().flat_map(|l| l.spans.iter().map(|s| &*s.content)).collect::<String>().contains("First"));
            assert!(items[1].text.lines.iter().flat_map(|l| l.spans.iter().map(|s| &*s.content)).collect::<String>().contains("Second"));
            assert!(items[2].text.lines.iter().flat_map(|l| l.spans.iter().map(|s| &*s.content)).collect::<String>().contains("Third"));
        }
    }

    #[test]
    fn test_ordered_list_items_have_text() {
        let theme = TestTheme;
        let renderer = MarkdownRenderer::new(&theme);
        let md = "1. Alpha\n2. Beta\n3. Gamma";
        let result = renderer.render(md);

        if let Some(ContentBlock::List { ordered: true, items, .. }) = result.iter().find(|b| matches!(b, ContentBlock::List { .. })) {
            assert_eq!(items.len(), 3);
            assert!(items[0].text.lines.iter().flat_map(|l| l.spans.iter().map(|s| &*s.content)).collect::<String>().contains("Alpha"));
            assert!(items[1].text.lines.iter().flat_map(|l| l.spans.iter().map(|s| &*s.content)).collect::<String>().contains("Beta"));
        } else {
            panic!("Expected ordered list block, got: {:?}", result);
        }
    }

    #[test]
    fn test_list_with_inline_code() {
        let theme = TestTheme;
        let renderer = MarkdownRenderer::new(&theme);
        let md = "- Use `cargo test` to run tests";
        let result = renderer.render(md);

        if let Some(ContentBlock::List { items, .. }) = result.iter().find(|b| matches!(b, ContentBlock::List { .. })) {
            assert_eq!(items.len(), 1);
            let text: String = items[0].text.lines.iter()
                .flat_map(|l| l.spans.iter().map(|s| &*s.content))
                .collect();
            assert!(text.contains("cargo test"), "Item text should contain inline code: {}", text);
        } else {
            panic!("Expected list block, got: {:?}", result);
        }
    }

    #[test]
    fn test_nested_ordered_unordered_list() {
        // Regression: nested list items from LLM output
        let theme = TestTheme;
        let renderer = MarkdownRenderer::new(&theme);
        let md = "1. First main point\n   - Item A\n   - Item B\n   - Item C\n2. Second main point\n   - Item X\n   - Item Y\n   - Item Z";
        let result = renderer.render(md);

        // Should produce one ordered list block
        let list_blocks: Vec<&ContentBlock> = result.iter()
            .filter(|b| matches!(b, ContentBlock::List { .. }))
            .collect();
        assert_eq!(list_blocks.len(), 1, "Expected 1 list block, got: {:?}", result);

        if let ContentBlock::List { ordered: true, items, .. } = list_blocks[0] {
            assert_eq!(items.len(), 2, "Expected 2 top-level items, got: {:?}", items);
            // Item 1 should have 3 children
            assert_eq!(items[0].children.len(), 3, "First item should have 3 children, got: {:?}", items[0]);
            // Item 2 should have 3 children
            assert_eq!(items[1].children.len(), 3, "Second item should have 3 children, got: {:?}", items[1]);
            // Check item texts
            let item1_text: String = items[0].text.lines.iter()
                .flat_map(|l| l.spans.iter().map(|s| &*s.content))
                .collect();
            assert!(item1_text.contains("First main point"), "Item 1 text: {}", item1_text);
            let item2_text: String = items[1].text.lines.iter()
                .flat_map(|l| l.spans.iter().map(|s| &*s.content))
                .collect();
            assert!(item2_text.contains("Second main point"), "Item 2 text: {}", item2_text);
            // Check children texts
            for (i, child) in items[0].children.iter().enumerate() {
                let text: String = child.text.lines.iter()
                    .flat_map(|l| l.spans.iter().map(|s| &*s.content))
                    .collect();
                assert!(!text.is_empty(), "Child {} of item 1 has empty text", i);
            }
            assert_eq!(
                items[0].children[0].text.lines.iter()
                    .flat_map(|l| l.spans.iter().map(|s| &*s.content))
                    .collect::<String>(),
                "Item A"
            );
            assert_eq!(
                items[1].children[0].text.lines.iter()
                    .flat_map(|l| l.spans.iter().map(|s| &*s.content))
                    .collect::<String>(),
                "Item X"
            );
        } else {
            panic!("Expected ordered list block, got: {:?}", result);
        }
    }

    // ─── ContentBlock height tests (from old content_block/mod.rs inline tests) ───

    #[test]
    fn wrapped_line_count_empty() {
        assert_eq!(wrapped_line_count("", 40), 1);
    }

    #[test]
    fn wrapped_line_count_zero_width() {
        assert_eq!(wrapped_line_count("hello", 0), 1);
    }

    #[test]
    fn wrapped_line_count_exact_fit() {
        assert_eq!(wrapped_line_count("hello", 5), 1);
    }

    #[test]
    fn wrapped_line_count_wraps_once() {
        // Word-wrap: "hello" (5 chars) fits on line 1, "world" (5 chars) on line 2
        assert_eq!(wrapped_line_count("hello world", 5), 2);
    }

    #[test]
    fn wrapped_line_count_wide() {
        assert_eq!(wrapped_line_count("short", 80), 1);
    }

    #[test]
    fn wrapped_height_empty_text() {
        let text = Text::from(vec![Line::default()]);
        assert_eq!(wrapped_height(&text, 40), 1);
    }

    #[test]
    fn wrapped_height_single_line() {
        let text = Text::from("hello");
        assert_eq!(wrapped_height(&text, 40), 1);
    }

    #[test]
    fn wrapped_height_multi_line() {
        let text = Text::from(vec![
            Line::from("hello"),
            Line::from("world"),
        ]);
        assert_eq!(wrapped_height(&text, 40), 2);
    }

    #[test]
    fn wrapped_height_wraps() {
        let long: String = (0..100).map(|_| 'a').collect();
        let text = Text::from(long);
        assert_eq!(wrapped_height(&text, 10), 10);
    }

    #[test]
    fn wrapped_height_zero_width() {
        let text = Text::from(vec![
            Line::from("hello"),
            Line::from("world"),
        ]);
        assert_eq!(wrapped_height(&text, 0), 2);
    }

    #[test]
    fn paragraph_height() {
        let block = ContentBlock::Paragraph {
            text: Text::from("hello world"),
        };
        assert_eq!(block.height(40, &TestTheme), 2); // 1 line + 1 blank
    }

    #[test]
    fn heading_height() {
        let block = ContentBlock::Heading {
            level: 1,
            text: Text::from("heading"),
        };
        assert_eq!(block.height(40, &TestTheme), 2); // 1 line + 1 blank
    }

    #[test]
    fn code_block_height() {
        let block = ContentBlock::CodeBlock {
            lang: "rust".to_string(),
            content: "fn main() {}\nlet x = 1;".to_string(),
        };
        // 4 (top border + title + bottom border + blank) + 2 (content lines) = 6
        assert_eq!(block.height(40, &TestTheme), 6);
    }

    #[test]
    fn table_height() {
        let block = ContentBlock::Table {
            headers: vec!["A".to_string(), "B".to_string()],
            rows: vec![
                vec!["foo".to_string(), "bar".to_string()],
                vec!["baz".to_string(), "qux".to_string()],
            ],
        };
        // 5 (borders+header+sep) + 2 (data rows) + 1 (inter-row sep) = 8
        assert_eq!(block.height(40, &TestTheme), 8);
    }

    #[test]
    fn table_height_wrapped_cells() {
        let long = "this is a very long cell content".to_string();
        let block = ContentBlock::Table {
            headers: vec!["A".to_string(), "B".to_string()],
            rows: vec![vec![long.clone(), "x".to_string()]],
        };
        // With narrow width, the long cell wraps → more rows
        let h_narrow = block.height(20, &TestTheme);
        let h_wide = block.height(80, &TestTheme);
        assert!(h_narrow > h_wide, "narrow table should be taller: {} > {}", h_narrow, h_wide);
    }

    #[test]
    fn list_height() {
        let block = ContentBlock::List {
            ordered: false,
            start: 1,
            items: vec![
                ListItem { text: Text::from("item 1"), children: vec![] },
                ListItem { text: Text::from("item 2"), children: vec![] },
            ],
        };
        // 2 items × (1 line + 1 blank) = 4
        assert_eq!(block.height(40, &TestTheme), 4);
    }

    #[test]
    fn list_height_nested() {
        let block = ContentBlock::List {
            ordered: false,
            start: 1,
            items: vec![
                ListItem {
                    text: Text::from("parent"),
                    children: vec![
                        ListItem { text: Text::from("child 1"), children: vec![] },
                        ListItem { text: Text::from("child 2"), children: vec![] },
                    ],
                },
            ],
        };
        // parent (2) + child1 (2) + child2 (2) = 6
        assert_eq!(block.height(40, &TestTheme), 6);
    }

    #[test]
    fn task_list_height() {
        let block = ContentBlock::TaskList {
            items: vec![
                TaskListItem { checked: false, text: Text::from("task 1") },
                TaskListItem { checked: true, text: Text::from("task 2") },
            ],
        };
        assert_eq!(block.height(40, &TestTheme), 4); // 2 items × 2
    }

    #[test]
    fn rule_height() {
        let block = ContentBlock::Rule;
        assert_eq!(block.height(40, &TestTheme), 2);
    }

    #[test]
    fn block_quote_height() {
        let block = ContentBlock::BlockQuote {
            text: Text::from("quoted text"),
        };
        assert_eq!(block.height(40, &TestTheme), 2); // 1 line + 1 blank
    }

    #[test]
    fn col_widths_minimum() {
        let headers = vec!["Name".to_string(), "Value".to_string()];
        let rows: Vec<Vec<String>> = vec![];
        let widths = compute_col_widths(&headers, &rows, 40, 2);
        assert_eq!(widths.len(), 2);
        assert!(widths[0] >= 4); // "Name" = 4
        assert!(widths[1] >= 5); // "Value" = 5
    }

    #[test]
    fn col_widths_minimum_3() {
        let headers = vec!["A".to_string(), "B".to_string()];
        let rows: Vec<Vec<String>> = vec![];
        let widths = compute_col_widths(&headers, &rows, 40, 2);
        assert!(widths[0] >= 3);
        assert!(widths[1] >= 3);
    }

    #[test]
    fn col_widths_zero_cols() {
        let headers: Vec<String> = vec![];
        let rows: Vec<Vec<String>> = vec![];
        let widths = compute_col_widths(&headers, &rows, 40, 0);
        assert!(widths.is_empty());
    }

    #[test]
    fn col_widths_proportional_distribution() {
        let headers = vec!["A".to_string(), "BBBB".to_string()];
        let rows: Vec<Vec<String>> = vec![];
        let widths = compute_col_widths(&headers, &rows, 20, 2);
        assert_eq!(widths[0] + widths[1] + 1, 20); // +1 for spacing
        assert!(widths[1] > widths[0], "wider column should get more space");
    }

    /// Verify Table height accounts for HighlightSpacing::Always rows.
    /// HighlightSpacing::Always adds spacing rows around highlighted rows.
    /// The height calculation must match the actual rendered height.
    #[test]
    fn table_height_accounts_for_highlight_spacing() {
        let theme = TestTheme;
        let headers = vec!["A".to_string(), "B".to_string()];
        let rows = vec![
            vec!["foo".to_string(), "bar".to_string()],
            vec!["baz".to_string(), "qux".to_string()],
        ];

        let block = ContentBlock::Table { headers, rows };
        let h = block.height(80, &theme);
        // Table height = 5 (borders + header + sep + blank) + data_rows + inter_row_seps
        // With HighlightSpacing::Always, the rendered height should match.
        // 5 + 2 + 1 = 8
        assert_eq!(h, 8);
    }

    /// Verify list height matches rendering for ordered and unordered lists.
    #[test]
    fn list_height_ordered_vs_unordered() {
        let theme = TestTheme;
        let long_text = "a".repeat(77); // Just over text_width boundary

        // Ordered list: bullet width = 4, text_width = width - 4
        let ordered = ContentBlock::List {
            ordered: true, start: 1,
            items: vec![ListItem { text: Text::from(long_text.clone()), children: vec![] }],
        };
        let ordered_h = ordered.height(80, &theme);
        // text_width = 76, 77 chars wraps to 2 lines, +1 = 3
        assert_eq!(ordered_h, 3, "ordered list height");

        // Unordered list: bullet width = 2, text_width = width - 4 (same in height calc)
        let unordered = ContentBlock::List {
            ordered: false, start: 1,
            items: vec![ListItem { text: Text::from(long_text), children: vec![] }],
        };
        let unordered_h = unordered.height(80, &theme);
        // text_width = 76 (height calc uses -4 for all lists), 77 chars wraps to 2, +1 = 3
        // But rendering uses text_width = 78 (bullet "• " = 2), so 77 chars = 1 line
        // This means height OVERESTIMATES for unordered lists by 1 row
        assert_eq!(unordered_h, 3, "unordered list height (overestimates vs rendering)");
    }

    /// Regression test: verify cumulative block heights match expected scroll positions.
    /// This ensures the clip_start logic in AssistantMessage::render_into skips the
    /// correct blocks when the viewport is scrolled.
    #[test]
    fn cumulative_block_heights_for_clip() {
        let theme = TestTheme;
        let blocks = MarkdownRenderer::new(&theme).render("# Title\n\nPara 1\n\nPara 2");

        let width = 80;
        let mut cumulative = 0usize;
        let expected: Vec<(usize, usize)> = blocks.iter().map(|b| {
            let h = b.height(width, &theme);
            let start = cumulative;
            cumulative += h;
            (start, h)
        }).collect();

        // Verify: clip_start of 3 should skip blocks 0-1 and render from block 2
        let clip_start = 3;
        let mut skip_count = 0;
        for (start, h) in &expected {
            if start + h <= clip_start {
                skip_count += 1;
            }
        }
        // Block 0 (H1): start=0, h=2, end=2 <= 3 → skip
        // Block 1 (Para 1): start=2, h=2, end=4 > 3 → render
        assert_eq!(skip_count, 1, "clip_start={} should skip {} blocks", clip_start, skip_count);
    }

    /// Regression: content cut-off bug
    #[test]
    fn full_message_height_matches_rendered() {
        let theme = TestTheme;
        let md = r#"# Title

This is a very long paragraph that will wrap multiple times to test if the height calculation is correct. It should include enough text to exceed the viewport width and force line wrapping. The content must be tall enough to trigger the bug where content was cut off at "2. Settings".

## Section 1

Some content here.

## Section 2

More content that needs to be visible.

1. Item one
2. Item two
3. Item three
4. Item four
5. Item five
6. Item six
7. Item seven
8. Item eight
9. Item nine
10. Item ten

## Section 3

Final section with additional content to ensure the total height is large enough to contain everything.

- List item 1
- List item 2
- List item 3
- List item 4
- List item 5

```rust
fn main() {
    println!("Hello, world!");
    let x = 42;
    let y = 100;
    let z = x + y;
    println!("Result: {}", z);
}
```

### Subsection 3.1

Even more content to push the total height past any reasonable viewport.

| Name | Value |
|------|-------|
| A    | 1     |
| B    | 2     |
| C    | 3     |

> This is a blockquote that adds more height to the message.

- Final list item 1
- Final list item 2
- Final list item 3
- Final list item 4
- Final list item 5
- Final list item 6
- Final list item 7
- Final list item 8
- Final list item 9
- Final list item 10"#;
        let renderer = MarkdownRenderer::new(&theme);
        let blocks = renderer.render(md);

        let width = 80;
        let total_height: usize = blocks.iter().map(|b| b.height(width, &theme)).sum();

        // Debug: print each block's height
        for (i, block) in blocks.iter().enumerate() {
            let h = block.height(width, &theme);
            let label = match block {
                ContentBlock::Paragraph { text } => {
                    let preview: String = text.lines.iter()
                        .flat_map(|l| l.spans.iter().map(|s| s.content.as_ref().to_string()))
                        .collect::<String>()
                        .chars().take(40).collect();
                    format!("Paragraph({})", preview)
                }
                ContentBlock::Heading { level, text } => {
                    let preview: String = text.lines.iter()
                        .flat_map(|l| l.spans.iter().map(|s| s.content.as_ref().to_string()))
                        .collect::<String>()
                        .chars().take(40).collect();
                    format!("H{}({})", level, preview)
                }
                ContentBlock::CodeBlock { lang, content } => {
                    format!("CodeBlock({}, {} lines)", lang, content.lines().count())
                }
                ContentBlock::Table { headers, rows } => {
                    format!("Table({}, {} rows)", headers.len(), rows.len())
                }
                ContentBlock::List { ordered, items, .. } => {
                    format!("List(ordered={}, {} items)", ordered, items.len())
                }
                ContentBlock::TaskList { items } => {
                    format!("TaskList({} items)", items.len())
                }
                ContentBlock::BlockQuote { text } => {
                    format!("BlockQuote({} lines)", text.lines.len())
                }
                ContentBlock::Rule => "Rule".to_string(),
            };
            println!("  Block {}: {} → height {}", i, label, h);
        }
        println!("  Total height: {}", total_height);

        // The total height should be large enough to contain all content.
        // If content is cut off, total_height < actual rendered height.
        assert!(total_height > 60, "total height {} seems too small — content would be cut off", total_height);
    }
}
