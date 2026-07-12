//! Table block rendering.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Paragraph, Widget, Wrap};

use crate::height::compute_col_widths;
use crate::theme::MdTheme;

/// Wrap text into lines that fit within the given width.
fn wrap_text(text: &str, max_width: usize) -> Vec<String> {
    if max_width == 0 {
        return vec![String::new()];
    }
    if text.chars().count() <= max_width {
        return vec![text.to_string()];
    }

    let mut lines = Vec::new();
    let mut current = String::new();
    for word in text.split_whitespace() {
        let word_len = word.chars().count();
        if current.is_empty() {
            // Word longer than max_width — break it
            if word_len > max_width {
                let chars: Vec<char> = word.chars().collect();
                let mut i = 0;
                while i < chars.len() {
                    let end = (i + max_width).min(chars.len());
                    let chunk: String = chars[i..end].iter().collect();
                    lines.push(chunk);
                    i += max_width;
                }
                current = String::new();
                continue;
            }
            current = word.to_string();
        } else if current.chars().count() + 1 + word_len <= max_width {
            current.push(' ');
            current.push_str(word);
        } else {
            lines.push(current);
            current = word.to_string();
        }
    }
    if !current.is_empty() {
        lines.push(current);
    }
    if lines.is_empty() {
        lines.push(String::new());
    }
    lines
}

pub fn render(
    headers: &[String],
    rows: &[Vec<String>],
    area: Rect,
    buf: &mut Buffer,
    theme: &dyn MdTheme,
) {
    if area.width < 4 || area.height < 2 {
        return;
    }

    let num_cols = headers.len().max(rows.iter().map(|r| r.len()).max().unwrap_or(0));
    if num_cols == 0 {
        return;
    }

    // Border overhead: "│ " + (n-1)*" │ " + " │" = 3n+1
    let border_overhead = 3 * num_cols + 1;
    let available_for_cells = (area.width as usize).saturating_sub(border_overhead);
    if available_for_cells < num_cols {
        let para = Paragraph::new(Text::from(headers.join(" | ")))
            .style(theme.text_style())
            .wrap(Wrap { trim: true });
        para.render(area, buf);
        return;
    }

    let col_widths = compute_col_widths(headers, rows, available_for_cells, num_cols);

    // Fill area with chat bg
    let bg_fill = Paragraph::new(Text::from(vec![])).style(theme.text_style());
    bg_fill.render(area, buf);

    let sep_style = Style::default().fg(theme.text_dim_color());

    // Build all table lines as styled Lines, then render
    let mut table_lines: Vec<Line<'static>> = Vec::new();

    // Top border: ┌─┬─┬─┐
    let parts: Vec<String> = col_widths.iter().take(num_cols)
        .map(|&w| "─".repeat(w)).collect();
    table_lines.push(Line::from(Span::styled(
        format!("┌─{}─┐", parts.join("─┬─")), sep_style
    )));

    // Helper: build content line spans with styled borders + cells
    let build_content_line = |cells: Vec<String>, cell_style: Style| -> Line<'static> {
        let mut spans: Vec<Span<'static>> = Vec::new();
        spans.push(Span::styled("│ ", sep_style));
        for (i, cell) in cells.into_iter().enumerate() {
            if i > 0 {
                spans.push(Span::styled(" │ ", sep_style));
            }
            spans.push(Span::styled(cell, cell_style));
        }
        spans.push(Span::styled(" │", sep_style));
        Line::from(spans)
    };

    // Header (with wrapping)
    let header_wrapped: Vec<Vec<String>> = headers.iter().take(num_cols)
        .zip(col_widths.iter().take(num_cols))
        .map(|(text, &w)| wrap_text(text, w)).collect();
    let header_height = header_wrapped.iter().map(|l| l.len()).max().unwrap_or(1).max(1);
    for line_idx in 0..header_height {
        let cells: Vec<String> = (0..num_cols).map(|ci| {
            let text = header_wrapped[ci].get(line_idx).cloned().unwrap_or_default();
            let pad = col_widths[ci].saturating_sub(text.chars().count());
            format!("{}{}", text, " ".repeat(pad))
        }).collect();
        table_lines.push(build_content_line(cells, theme.heading_style()));
    }

    // Separator: ├─┼─┼─┤
    table_lines.push(Line::from(Span::styled(
        format!("├─{}─┤", parts.join("─┼─")), sep_style
    )));

    // Data rows
    for (row_idx, row) in rows.iter().enumerate() {
        let cell_wrapped: Vec<Vec<String>> = (0..num_cols).map(|ci| {
            let text = row.get(ci).cloned().unwrap_or_default();
            wrap_text(&text, col_widths[ci])
        }).collect();
        let row_height = cell_wrapped.iter().map(|l| l.len()).max().unwrap_or(1).max(1);
        for line_idx in 0..row_height {
            let cells: Vec<String> = (0..num_cols).map(|ci| {
                let text = cell_wrapped[ci].get(line_idx).cloned().unwrap_or_default();
                let pad = col_widths[ci].saturating_sub(text.chars().count());
                format!("{}{}", text, " ".repeat(pad))
            }).collect();
            table_lines.push(build_content_line(cells, theme.text_style()));
        }
        if row_idx < rows.len() - 1 {
            table_lines.push(Line::from(Span::styled(
                format!("├─{}─┤", parts.join("─┼─")), sep_style
            )));
        }
    }

    // Bottom border: └─┴─┴─┘
    table_lines.push(Line::from(Span::styled(
        format!("└─{}─┘", parts.join("─┴─")), sep_style
    )));

    // Render lines
    for (i, line) in table_lines.iter().take(area.height as usize).enumerate() {
        let text = Text::from(line.clone());
        let para = Paragraph::new(text);
        let line_area = Rect { x: area.x, y: area.y + i as u16, width: area.width, height: 1 };
        para.render(line_area, buf);
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

    // ─── wrap_text ───

    #[test]
    fn wrap_text_fits() {
        assert_eq!(wrap_text("hello", 10), vec!["hello".to_string()]);
    }

    #[test]
    fn wrap_text_exact_fit() {
        assert_eq!(wrap_text("hello", 5), vec!["hello".to_string()]);
    }

    #[test]
    fn wrap_text_wraps() {
        let lines = wrap_text("hello world", 5);
        assert_eq!(lines, vec!["hello", "world"]);
    }

    #[test]
    fn wrap_text_word_longer_than_width() {
        let lines = wrap_text("supercalifragilistic", 5);
        assert_eq!(lines.len(), 4); // ceil(18/5) = 4
        assert_eq!(lines[0], "super");
        assert_eq!(lines[1], "calif");
    }

    #[test]
    fn wrap_text_empty_string() {
        let lines = wrap_text("", 10);
        assert_eq!(lines, vec!["".to_string()]);
    }

    #[test]
    fn wrap_text_zero_width() {
        let lines = wrap_text("hello", 0);
        assert_eq!(lines, vec![String::new()]);
    }

    #[test]
    fn wrap_text_single_word() {
        let lines = wrap_text("single", 20);
        assert_eq!(lines, vec!["single".to_string()]);
    }

    #[test]
    fn wrap_text_unicode() {
        let lines = wrap_text("日本语", 2);
        assert_eq!(lines.len(), 2); // 3 chars / 2 = 2 lines
    }

    // ─── render ───

    #[test]
    fn render_simple_table() {
        let headers = vec!["A".into(), "B".into()];
        let rows = vec![vec!["1".into(), "2".into()]];
        let area = Rect::new(0, 0, 20, 10);
        let mut buf = Buffer::empty(area);
        render(&headers, &rows, area, &mut buf, &TestTheme);
        let got: String = buf.content.iter().map(|c| c.symbol().chars().next().unwrap_or(' ')).collect();
        assert!(got.contains("A"), "Should contain header A");
        assert!(got.contains("B"), "Should contain header B");
        assert!(got.contains("1"), "Should contain data 1");
    }

    #[test]
    fn render_table_with_wrapped_cells() {
        let headers = vec!["Name".into(), "Value".into()];
        let rows = vec![vec!["Very long cell text".into(), "x".into()]];
        let area = Rect::new(0, 0, 30, 10);
        let mut buf = Buffer::empty(area);
        render(&headers, &rows, area, &mut buf, &TestTheme);
        let got: String = buf.content.iter().map(|c| c.symbol().chars().next().unwrap_or(' ')).collect();
        assert!(got.contains("Very long"));
    }

    #[test]
    fn render_table_narrow_area() {
        let headers = vec!["A".into(), "B".into()];
        let rows = vec![vec!["1".into(), "2".into()]];
        let area = Rect::new(0, 0, 3, 5); // Too narrow
        let mut buf = Buffer::empty(area);
        render(&headers, &rows, area, &mut buf, &TestTheme);
        // Should not panic — falls back to paragraph
    }

    #[test]
    fn render_table_single_column() {
        let headers = vec!["Col".into()];
        let rows = vec![vec!["val".into()]];
        let area = Rect::new(0, 0, 15, 10);
        let mut buf = Buffer::empty(area);
        render(&headers, &rows, area, &mut buf, &TestTheme);
        let got: String = buf.content.iter().map(|c| c.symbol().chars().next().unwrap_or(' ')).collect();
        assert!(got.contains("Col"));
        assert!(got.contains("val"));
    }

    #[test]
    fn render_table_multiple_rows_with_separators() {
        let headers = vec!["A".into(), "B".into()];
        let rows = vec![
            vec!["1".into(), "2".into()],
            vec!["3".into(), "4".into()],
        ];
        let area = Rect::new(0, 0, 20, 15);
        let mut buf = Buffer::empty(area);
        render(&headers, &rows, area, &mut buf, &TestTheme);
        let got: String = buf.content.iter().map(|c| c.symbol().chars().next().unwrap_or(' ')).collect();
        // Should have inter-row separator (├)
        assert!(got.contains('├'), "Should have inter-row separator");
    }

    #[test]
    fn render_table_no_rows() {
        let headers = vec!["A".into(), "B".into()];
        let rows: Vec<Vec<String>> = vec![];
        let area = Rect::new(0, 0, 20, 10);
        let mut buf = Buffer::empty(area);
        render(&headers, &rows, area, &mut buf, &TestTheme);
        let got: String = buf.content.iter().map(|c| c.symbol().chars().next().unwrap_or(' ')).collect();
        assert!(got.contains("A"));
        assert!(got.contains('┌'), "Should have top border");
        assert!(got.contains('└'), "Should have bottom border");
    }

    #[test]
    fn render_table_zero_area() {
        let headers = vec!["A".into()];
        let rows: Vec<Vec<String>> = vec![];
        let area = Rect::new(0, 0, 0, 0);
        let mut buf = Buffer::empty(area);
        render(&headers, &rows, area, &mut buf, &TestTheme);
        // Should not panic
    }

    #[test]
    fn render_table_border_characters() {
        let headers = vec!["A".into()];
        let rows = vec![vec!["1".into()]];
        let area = Rect::new(0, 0, 15, 10);
        let mut buf = Buffer::empty(area);
        render(&headers, &rows, area, &mut buf, &TestTheme);
        let got: String = buf.content.iter().map(|c| c.symbol().chars().next().unwrap_or(' ')).collect();
        assert!(got.contains('┌'), "Top-left corner");
        assert!(got.contains('┐'), "Top-right corner");
        assert!(got.contains('└'), "Bottom-left corner");
        assert!(got.contains('┘'), "Bottom-right corner");
        assert!(got.contains('│'), "Vertical border");
        assert!(got.contains('─'), "Horizontal border");
    }
}
