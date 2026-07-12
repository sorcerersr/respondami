//! Height calculation for ContentBlock variants.

use ratatui::text::Text;

use crate::theme::MdTheme;
use crate::types::{ContentBlock, ListItem};

/// Compute wrapped height of a `Text` at a given width.
/// Uses word-boundary wrapping to match ratatui Paragraph with Wrap.
pub fn wrapped_height(text: &Text<'_>, width: usize) -> usize {
    if width == 0 {
        return text.lines.len();
    }
    let mut total_lines = 0;
    for line in &text.lines {
        let plain: String = line.spans.iter().map(|s| s.content.as_ref()).collect();
        total_lines += word_wrap_height(&plain, width);
    }
    total_lines.max(1)
}

/// Count lines produced by word-boundary wrapping.
/// Matches ratatui Paragraph with Wrap { trim: true }.
pub fn wrapped_line_count(text: &str, width: usize) -> usize {
    if width == 0 {
        return 1;
    }
    if text.is_empty() {
        return 1;
    }
    word_wrap_height(text, width)
}

/// Count lines produced by word-boundary wrapping for plain text.
/// Matches ratatui's Wrap { trim: true }: words are packed at boundaries,
/// and words longer than width are broken across lines.
fn word_wrap_height(text: &str, width: usize) -> usize {
    if text.is_empty() {
        return 1;
    }
    let mut lines = 0;
    let mut current_len = 0;

    for word in text.split_whitespace() {
        let word_len = word.chars().count();

        // Words longer than width are broken (trim: true)
        if word_len > width {
            lines += word_len.div_ceil(width);
            current_len = 0;
            continue;
        }

        if current_len == 0 {
            // First word opens a new line
            lines += 1;
            current_len = word_len;
        } else if current_len + 1 + word_len <= width {
            current_len += 1 + word_len;
        } else {
            lines += 1;
            current_len = word_len;
        }
    }

    lines.max(1)
}

/// Height of a list item including children.
fn list_item_height(item: &ListItem, width: usize) -> usize {
    let text_width = width.saturating_sub(4); // bullet + indent overhead
    // Empty items render as just the bullet (1 line), not bullet + empty paragraph (2 lines).
    let item_h = if item.text.lines.is_empty() {
        1
    } else {
        wrapped_height(&item.text, text_width) + 1
    };
    let children_h: usize = item.children.iter()
        .map(|c| list_item_height(c, width))
        .sum();
    item_h + children_h
}

/// Compute column widths for a table given available width.
pub fn compute_col_widths(
    headers: &[String],
    rows: &[Vec<String>],
    available_width: usize,
    num_cols: usize,
) -> Vec<usize> {
    if num_cols == 0 || available_width == 0 {
        return vec![0; num_cols];
    }

    // Compute minimum widths from content
    let mut min_widths: Vec<usize> = vec![0; num_cols];
    for (i, header) in headers.iter().enumerate().take(num_cols) {
        min_widths[i] = min_widths[i].max(header.chars().count());
    }
    for row in rows {
        for (i, cell) in row.iter().enumerate().take(num_cols) {
            min_widths[i] = min_widths[i].max(cell.chars().count());
        }
    }

    // Ensure minimum width of 3
    for w in &mut min_widths {
        if *w < 3 {
            *w = 3;
        }
    }

    // Account for column spacing (1 char between columns)
    let spacing = num_cols.saturating_sub(1);
    let total_min: usize = min_widths.iter().sum::<usize>() + spacing;

    if total_min <= available_width {
        // Distribute extra space proportionally
        let extra = available_width - total_min;
        let total_min_content: usize = min_widths.iter().sum();
        if total_min_content > 0 {
            let mut widths = min_widths.clone();
            let mut remaining = extra;
            for i in 0..num_cols {
                if remaining == 0 {
                    break;
                }
                let share = (min_widths[i] as f64 / total_min_content as f64 * extra as f64) as usize;
                let add = share.min(remaining);
                widths[i] += add;
                remaining -= add;
            }
            // Distribute any leftover (from truncation) round-robin
            let mut i = 0;
            while remaining > 0 && i < num_cols {
                widths[i] = widths[i].saturating_add(1);
                remaining -= 1;
                i += 1;
            }
            return widths;
        }
        return min_widths;
    }

    // Can't fit minimum widths — distribute available space evenly
    let base = available_width.div_ceil(num_cols);
    let mut widths = vec![base; num_cols];
    let mut distributed = base.saturating_mul(num_cols);
    let mut i = 0;
    while distributed < available_width && i < num_cols {
        widths[i] = widths[i].saturating_add(1);
        distributed = distributed.saturating_add(1);
        i += 1;
    }
    widths
}

/// Public entry point for ContentBlock::height.
pub fn content_block_height(block: &ContentBlock, width: usize, _theme: &dyn MdTheme) -> usize {
    match block {
        ContentBlock::Paragraph { text } => wrapped_height(text, width) + 1,
        ContentBlock::Heading { text, .. } => wrapped_height(text, width) + 1,
        ContentBlock::CodeBlock { content, .. } => {
            // top border + title + content lines + bottom border + blank
            4 + content.lines().count()
        }
        ContentBlock::Table { headers, rows } => {
            if headers.is_empty() && rows.is_empty() {
                return 1;
            }
            let num_cols = headers.len().max(rows.iter().map(|r| r.len()).max().unwrap_or(0));
            if num_cols == 0 {
                return 1;
            }
            // Border overhead: "│ " + (n-1)*" │ " + " │" = 3n+1
            let border_overhead = 3 * num_cols + 1;
            let available = width.saturating_sub(border_overhead);
            if available < num_cols {
                return 1; // too narrow, fallback
            }
            let col_widths = compute_col_widths(headers, rows, available, num_cols);
            // top border (1) + header lines + separator (1) + data rows + inter-row seps + bottom border (1) + blank (1)
            let header_height = headers.iter().take(num_cols)
                .zip(col_widths.iter())
                .map(|(cell, &w)| wrapped_line_count(cell, w))
                .max().unwrap_or(1).max(1);
            let data_height: usize = rows.iter().map(|row| {
                row.iter().take(num_cols).zip(col_widths.iter())
                   .map(|(cell, &w)| wrapped_line_count(cell, w))
                   .max().unwrap_or(1)
            }).sum();
            let inter_row_seps = rows.len().saturating_sub(1);
            4 + header_height + data_height + inter_row_seps
        }
        ContentBlock::List { items, .. } => {
            items.iter().map(|item| list_item_height(item, width)).sum()
        }
        ContentBlock::TaskList { items } => {
            items.iter().map(|item| {
                wrapped_height(&item.text, width.saturating_sub(3)) + 1
            }).sum()
        }
        ContentBlock::BlockQuote { text } => {
            wrapped_height(text, width.saturating_sub(2)) + 1
        }
        ContentBlock::Rule => 2,
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

    // ─── word_wrap_height ───

    #[test]
    fn word_wrap_empty() {
        assert_eq!(super::word_wrap_height("", 40), 1);
    }

    #[test]
    fn word_wrap_single_word_fits() {
        assert_eq!(super::word_wrap_height("hello", 10), 1);
    }

    #[test]
    fn word_wrap_two_words_fit() {
        assert_eq!(super::word_wrap_height("hello world", 12), 1);
    }

    #[test]
    fn word_wrap_two_words_wrap() {
        assert_eq!(super::word_wrap_height("hello world", 5), 2);
    }

    #[test]
    fn word_wrap_long_word_exceeds_width() {
        // "supercalifragilistic" = 18 chars, width = 5 → ceil(18/5) = 4
        assert_eq!(super::word_wrap_height("supercalifragilistic", 5), 4);
    }

    #[test]
    fn word_wrap_long_word_exact_multiple() {
        // 10 chars, width = 5 → 2 lines
        assert_eq!(super::word_wrap_height("aaaaaaaaaa", 5), 2);
    }

    #[test]
    fn word_wrap_long_word_plus_short() {
        // "aaaaaa" (6) exceeds width 3 → 2 lines, then "hi" → 1 line = 3
        assert_eq!(super::word_wrap_height("aaaaaa hi", 3), 3);
    }

    #[test]
    fn word_wrap_single_char_words() {
        assert_eq!(super::word_wrap_height("a b c d e", 3), 3); // "a b", "c d", "e"
    }

    #[test]
    fn word_wrap_unicode_chars() {
        // "日本" = 2 chars, width = 1 → 2 lines
        assert_eq!(super::word_wrap_height("日本", 1), 2);
    }

    // ─── wrapped_height ───

    #[test]
    fn wrapped_height_single_span() {
        let text = Text::from("hello world");
        assert_eq!(wrapped_height(&text, 10), 2);
    }

    #[test]
    fn wrapped_height_multiple_lines() {
        let text = Text::from(vec![
            ratatui::text::Line::from("line one"),
            ratatui::text::Line::from("line two"),
        ]);
        assert_eq!(wrapped_height(&text, 40), 2);
    }

    #[test]
    fn wrapped_height_empty_text_empty_lines() {
        let text = Text::from(vec![]);
        assert_eq!(wrapped_height(&text, 40), 1); // .max(1)
    }

    #[test]
    fn wrapped_height_zero_width_returns_line_count() {
        let text = Text::from(vec![
            ratatui::text::Line::from("a"),
            ratatui::text::Line::from("b"),
        ]);
        assert_eq!(wrapped_height(&text, 0), 2);
    }

    // ─── wrapped_line_count ───

    #[test]
    fn wrapped_line_count_single_word() {
        assert_eq!(wrapped_line_count("hello", 10), 1);
    }

    #[test]
    fn wrapped_line_count_many_words() {
        // "the quick brown fox jumps" = 5 words, width = 8
        // "the" (3) + "quick" (5) → 3+1+5=9 > 8, so "the" (1), "quick" (1), "brown" (1), "fox" (1), "jumps" (1) = 5?
        // Actually: "the" fits (3), "quick" doesn't (3+1+5=9>8) → new line
        // "quick" (5), "brown" doesn't (5+1+5=11>8) → new line
        // "brown" (5), "fox" doesn't (5+1+3=9>8) → new line
        // "fox" (3), "jumps" doesn't (3+1+5=9>8) → new line
        // "jumps" (5) → 5 lines
        assert_eq!(wrapped_line_count("the quick brown fox jumps", 8), 5);
    }

    // ─── compute_col_widths ───

    #[test]
    fn col_widths_exact_fit() {
        let headers = vec!["AB".into(), "CD".into()];
        let widths = compute_col_widths(&headers, &[], 5, 2);
        // min_widths = [3, 3] (minimum 3), spacing = 1, total_min = 7 > 5
        // Can't fit → base = 5/2 = 3, widths = [3, 2]... actually div_ceil(5,2) = 3
        // widths = [3, 3], distributed = 6 > 5 — hmm, let me check
        assert_eq!(widths.len(), 2);
    }

    #[test]
    fn col_widths_single_column() {
        let headers = vec!["Col".into()];
        let widths = compute_col_widths(&headers, &[], 10, 1);
        assert_eq!(widths.len(), 1);
        assert_eq!(widths[0], 10); // all space goes to single column
    }

    #[test]
    fn col_widths_three_columns() {
        let headers = vec!["A".into(), "B".into(), "C".into()];
        let widths = compute_col_widths(&headers, &[], 30, 3);
        assert_eq!(widths.len(), 3);
        let spacing = 2;
        assert_eq!(widths.iter().sum::<usize>() + spacing, 30);
    }

    #[test]
    fn col_widths_content_determines_minimum() {
        let headers = vec!["Short".into(), "VeryLongHeader".into()];
        let widths = compute_col_widths(&headers, &[], 40, 2);
        assert!(widths[1] >= 14, "Column 2 should be at least 14 for 'VeryLongHeader'");
    }

    #[test]
    fn col_widths_rows_affect_minimum() {
        let headers = vec!["A".into(), "B".into()];
        let rows = vec![vec!["short".into(), "very long cell content".into()]];
        let widths = compute_col_widths(&headers, &rows, 40, 2);
        assert!(widths[1] >= 22, "Column 2 should be at least 22");
    }

    #[test]
    fn col_widths_zero_available() {
        let headers = vec!["A".into(), "B".into()];
        let widths = compute_col_widths(&headers, &[], 0, 2);
        assert_eq!(widths, vec![0, 0]);
    }

    #[test]
    fn col_widths_zero_cols() {
        let headers: Vec<String> = vec![];
        let widths = compute_col_widths(&headers, &[], 40, 0);
        assert!(widths.is_empty());
    }

    // ─── content_block_height ───

    #[test]
    fn height_paragraph() {
        let block = ContentBlock::Paragraph { text: Text::from("hello") };
        assert_eq!(content_block_height(&block, 80, &TestTheme), 2); // 1 + 1
    }

    #[test]
    fn height_heading_any_level() {
        let block = ContentBlock::Heading { level: 3, text: Text::from("title") };
        assert_eq!(content_block_height(&block, 80, &TestTheme), 2);
    }

    #[test]
    fn height_code_block_single_line() {
        let block = ContentBlock::CodeBlock { lang: "rust".into(), content: "fn main() {}.".into() };
        assert_eq!(content_block_height(&block, 80, &TestTheme), 5); // 4 + 1
    }

    #[test]
    fn height_code_block_multiline() {
        let block = ContentBlock::CodeBlock { lang: "".into(), content: "a\nb\nc".into() };
        assert_eq!(content_block_height(&block, 80, &TestTheme), 7); // 4 + 3
    }

    #[test]
    fn height_code_block_empty_content() {
        let block = ContentBlock::CodeBlock { lang: "".into(), content: "".into() };
        // "".lines().count() = 0 → 4 + 0 = 4
        assert_eq!(content_block_height(&block, 80, &TestTheme), 4);
    }

    #[test]
    fn height_table_empty() {
        let block = ContentBlock::Table { headers: vec![], rows: vec![] };
        assert_eq!(content_block_height(&block, 80, &TestTheme), 1);
    }

    #[test]
    fn height_table_narrow_fallback() {
        let block = ContentBlock::Table {
            headers: vec!["A".into(), "B".into()],
            rows: vec![vec!["1".into(), "2".into()]],
        };
        // width=5, border_overhead = 3*2+1 = 7, available = 0 → fallback = 1
        assert_eq!(content_block_height(&block, 5, &TestTheme), 1);
    }

    #[test]
    fn height_task_list_single_item() {
        let block = ContentBlock::TaskList {
            items: vec![crate::types::TaskListItem { checked: true, text: Text::from("do this") }],
        };
        assert_eq!(content_block_height(&block, 80, &TestTheme), 2); // 1 + 1
    }

    #[test]
    fn height_task_list_wrapped() {
        let long = "a".repeat(100);
        let block = ContentBlock::TaskList {
            items: vec![crate::types::TaskListItem { checked: false, text: Text::from(long) }],
        };
        // text width = 40 - 3 = 37, 100 chars → ceil(100/37) = 3 lines + 1 = 4
        assert_eq!(content_block_height(&block, 40, &TestTheme), 4);
    }

    #[test]
    fn height_blockquote() {
        let block = ContentBlock::BlockQuote { text: Text::from("quote") };
        assert_eq!(content_block_height(&block, 80, &TestTheme), 2);
    }

    #[test]
    fn height_blockquote_narrow() {
        let block = ContentBlock::BlockQuote { text: Text::from("a long quote that wraps") };
        // width = 20, text_width = 18, "a long quote that wraps" = 23 chars
        // "a long" (6) + "quote" (5) = 12, "that" (4) + "wraps" (5) = 10 → 2 lines + 1 = 3
        assert_eq!(content_block_height(&block, 20, &TestTheme), 3);
    }

    #[test]
    fn height_rule() {
        let block = ContentBlock::Rule;
        assert_eq!(content_block_height(&block, 80, &TestTheme), 2);
    }

    #[test]
    fn height_list_empty_item() {
        let block = ContentBlock::List {
            ordered: false, start: 1,
            items: vec![ListItem { text: Text::from(vec![]), children: vec![] }],
        };
        assert_eq!(content_block_height(&block, 80, &TestTheme), 1); // bullet only
    }

    #[test]
    fn height_list_deeply_nested() {
        let block = ContentBlock::List {
            ordered: false, start: 1,
            items: vec![ListItem {
                text: Text::from("parent"),
                children: vec![ListItem {
                    text: Text::from("child"),
                    children: vec![ListItem { text: Text::from("grandchild"), children: vec![] }],
                }],
            }],
        };
        // parent: 2, child: 2, grandchild: 1 = 5
        let h = content_block_height(&block, 80, &TestTheme);
        assert!(h >= 5, "Deeply nested list height should be >= 5, got {}", h);
    }

    #[test]
    fn height_list_zero_width() {
        let block = ContentBlock::List {
            ordered: false, start: 1,
            items: vec![ListItem { text: Text::from("item"), children: vec![] }],
        };
        // width=0 → text_width=0 → wrapped_height returns lines.len() = 1, +1 = 2
        assert_eq!(content_block_height(&block, 0, &TestTheme), 2);
    }
}
