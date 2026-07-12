//! Editor renderer — input area rendering with autocomplete support.
//!
//! Renders the input buffer with soft-wrap, block cursor, placeholder text,
//! and visual-line cursor positioning. Autocomplete overlays are rendered in
//! `layout.rs`.

use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use ratatui::Frame;

use super::super::app::App;
use super::super::mode::AutocompleteMode;
use super::super::theme::Theme;
use super::wrap::{build_visual_lines, cursor_visual_pos};

/// Maximum number of items visible in the file autocomplete popup.
pub const FILE_POPUP_PAGE_SIZE: usize = 10;

/// Editor renderer with autocomplete support.
#[derive(Debug)]
pub struct EditorRenderer;

impl EditorRenderer {
    pub fn render(frame: &mut Frame, area: Rect, app: &App, theme: &Theme) {
        let input = &app.editor.input_buffer;
        let cursor_pos = app.editor.cursor_pos;

        match &app.editor.autocomplete_mode {
            AutocompleteMode::None
            | AutocompleteMode::File { .. }
            | AutocompleteMode::Skill { .. } => {
                // Both overlays rendered in layout.rs. Here we just render input.
                Self::render_input_only(frame, area, input, cursor_pos, theme);
            }
        }
    }

    fn render_input_only(frame: &mut Frame, area: Rect, input: &str, cursor_pos: usize, theme: &Theme) {
        let total_height = area.height as usize;
        if total_height < 1 {
            return;
        }

        let wrap_width = area.width as usize;
        let text_height = total_height; // Full area available (no separator lines)

        // Build visual lines from input (accounts for soft-wrapping)
        let visual_lines = build_visual_lines(input, wrap_width);
        let total_visual = visual_lines.len().max(1); // At least 1 visual line (empty input)
        let visible_text = total_visual.clamp(1, text_height);

        // Compute cursor's visual position
        let (cursor_vis_line, cursor_vis_col) = cursor_visual_pos(input, cursor_pos, wrap_width);

        // Compute visible window: center on cursor visual line, clamped to bounds
        let start_line = if total_visual > visible_text {
            let half = visible_text / 2;
            let mut start = cursor_vis_line.saturating_sub(half);
            // Clamp so cursor is always visible
            if start + visible_text > total_visual {
                start = total_visual - visible_text;
            }
            start
        } else {
            0
        };
        let end_line = (start_line + visible_text).min(total_visual);

        let mut rendered_lines = Vec::new();

        // Placeholder text shown when input is empty
        const PLACEHOLDER: &str = "Type a message…";
        let normal_style = Style::default().fg(theme.text).bg(theme.bg_light);
        let placeholder_style = Style::default().fg(theme.text_dim).bg(theme.bg_light);
        let block_style = Style::default()
            .fg(theme.bg_light)
            .bg(theme.text)
            .add_modifier(ratatui::style::Modifier::BOLD);

        // Text lines with cursor bar
        for (i, &(line_idx, seg_start, seg_end)) in visual_lines.iter().enumerate().take(end_line).skip(start_line) {
            // Get the text of this logical line
            let line_text = input.split('\n').nth(line_idx).unwrap_or("");
            let segment_text = &line_text[seg_start..seg_end];

            // Replace empty segment with placeholder text on the first visual line
            let display_text = if input.is_empty() && i == 0 && segment_text.is_empty() {
                PLACEHOLDER
            } else {
                segment_text
            };

            // Is this the cursor's visual line?
            if i == cursor_vis_line {
                // Render with block cursor at visual column position
                let (before, rest) = split_at_display_width(display_text, cursor_vis_col);

                let mut spans = Vec::new();
                if !before.is_empty() {
                    spans.push(Span::styled(
                        before,
                        if input.is_empty() { placeholder_style } else { normal_style },
                    ));
                }

                if rest.is_empty() {
                    // Cursor at end of segment — show block on a space
                    spans.push(Span::styled(" ".to_string(), block_style));
                } else {
                    // Block cursor: highlight the character under the cursor
                    let ch = rest.chars().next().unwrap();
                    spans.push(Span::styled(ch.to_string(), block_style));
                    let ch_byte_len = ch.len_utf8();
                    if ch_byte_len < rest.len() {
                        spans.push(Span::styled(
                            rest[ch_byte_len..].to_string(),
                            if input.is_empty() { placeholder_style } else { normal_style },
                        ));
                    }
                }
                rendered_lines.push(Line::from(spans));
            } else {
                let style = if input.is_empty() { placeholder_style } else { normal_style };
                rendered_lines.push(Line::from(Span::styled(
                    display_text.to_string(),
                    style,
                )));
            }
        }

        // Fill remaining text rows if fewer visual lines than visible height
        while rendered_lines.len() < total_height {
            rendered_lines.push(Line::from(Span::styled(
                String::new(),
                Style::default().fg(theme.text).bg(theme.bg_light),
            )));
        }

        // bg_light distinguishes input area from chat area above.
        let paragraph = Paragraph::new(rendered_lines)
            .style(Style::default().bg(theme.bg_light));
        frame.render_widget(paragraph, area);
    }
}

/// Split `text` into `(before, after)` at `display_width_col`.
///
/// `before` has display width ≤ `col`, `after` contains the rest.
/// If `col == 0`, returns `("", text)`.
/// If `col >= text.width()`, returns `(text, "")`.
fn split_at_display_width(text: &str, col: usize) -> (String, String) {
    let text_width = text.width();
    if col == 0 {
        return (String::new(), text.to_string());
    }
    if col >= text_width {
        return (text.to_string(), String::new());
    }

    let mut acc = 0;
    for (idx, ch) in text.char_indices() {
        let ch_width = ch.width().unwrap_or(0);
        if acc + ch_width > col {
            return (text[..idx].to_string(), text[idx..].to_string());
        }
        acc += ch_width;
    }

    // Should not reach here if col < text_width, but handle gracefully
    (text.to_string(), String::new())
}
