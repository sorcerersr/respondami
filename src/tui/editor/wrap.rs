//! Text wrapping utilities for the editor.
//!
//! Provides `wrap_line()` for soft-wrapping logical lines into visual segments,
//! `build_visual_lines()` for building the full visual line list, and
//! `cursor_visual_pos()` for mapping byte positions to visual (wrapped) coordinates.

use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

/// Wrap a single logical line into visual segments that fit `max_width` display columns.
///
/// Returns a `Vec` of `(byte_start, byte_end)` ranges within the original line string.
/// Each segment's display width is ≤ `max_width`. Multi-byte characters are never split.
/// An empty line returns a single `(0, 0)` segment.
/// Returns empty vec when `max_width == 0`.
#[must_use]
pub fn wrap_line(line: &str, max_width: usize) -> Vec<(usize, usize)> {
    if max_width == 0 {
        return Vec::new();
    }

    if line.is_empty() {
        return vec![(0, 0)];
    }

    let mut segments = Vec::new();
    let mut seg_start = 0;
    let mut current_width = 0;

    for (idx, ch) in line.char_indices() {
        let ch_width = ch.width().unwrap_or(0);
        if current_width > 0 && current_width + ch_width > max_width {
            // Close current segment at this char's start
            segments.push((seg_start, idx));
            seg_start = idx;
            current_width = ch_width;
        } else {
            current_width += ch_width;
        }
    }

    // Final segment
    segments.push((seg_start, line.len()));

    segments
}

/// Build a flat list of all visual lines from the input buffer.
///
/// Each entry is `(logical_line_index, byte_start, byte_end)` where `byte_start` and
/// `byte_end` are offsets within the original line (obtained via `input.split('\n')`).
///
/// A logical line that doesn't exceed `wrap_width` produces exactly one visual line.
/// A longer logical line produces multiple visual lines.
#[must_use]
pub fn build_visual_lines(input: &str, wrap_width: usize) -> Vec<(usize, usize, usize)> {
    let mut visual_lines = Vec::new();

    for (line_idx, line) in input.split('\n').enumerate() {
        let segments = wrap_line(line, wrap_width);
        for (seg_start, seg_end) in segments {
            visual_lines.push((line_idx, seg_start, seg_end));
        }
    }

    visual_lines
}

/// Map a cursor byte position to `(visual_line_index, visual_column_display_width)`.
///
/// The `visual_line_index` is a flat index into the list returned by `build_visual_lines`.
/// The `visual_column` is the display-width offset of the cursor within that visual line.
#[must_use]
pub fn cursor_visual_pos(input: &str, pos: usize, wrap_width: usize) -> (usize, usize) {
    let clamped = pos.min(input.len());

    // Find which logical line the cursor is on
    let (logical_line, col_in_line) = cursor_line_col(input, clamped);

    // Get the text of that logical line
    let line_text = input.split('\n').nth(logical_line).unwrap_or("");

    // Wrap the line
    let segments = wrap_line(line_text, wrap_width);

    // Count visual lines before this logical line
    let mut visual_line_offset = 0;
    for prev_line in input.split('\n').take(logical_line) {
        visual_line_offset += wrap_line(prev_line, wrap_width).len();
    }

    // Find the segment containing col_in_line
    // Defensive: round col_in_line down to nearest char boundary.
    // cursor_up/cursor_down should already do this, but protect against edge cases.
    let safe_col = last_char_boundary(line_text, col_in_line);

    for (i, &(seg_start, seg_end)) in segments.iter().enumerate() {
        if safe_col <= seg_end {
            let vis_line = visual_line_offset + i;
            // Visual col: display width of text from seg_start to safe_col
            let vis_col = line_text[seg_start..safe_col].width();
            return (vis_line, vis_col);
        }
    }

    // Cursor is past all segments (shouldn't happen, but handle)
    let vis_line = visual_line_offset + segments.len().saturating_sub(1);
    let last_seg = segments.last().map_or(0, |(_, end)| *end);
    let vis_col = line_text[..last_seg].width();
    (vis_line, vis_col)
}

/// Round `pos` down to the nearest valid char boundary in `text`.
pub(crate) fn last_char_boundary(text: &str, pos: usize) -> usize {
    let pos = pos.min(text.len());
    if pos == 0 {
        return 0;
    }
    // Find the last char whose start + len <= pos
    text.char_indices()
        .take_while(|&(idx, ch)| idx + ch.len_utf8() <= pos)
        .last()
        .map_or(0, |(idx, ch)| idx + ch.len_utf8())
}

/// Convert a byte-offset cursor position to (`line_index`, `column_byte_offset`).
/// Lines are split on '\n'. The column is the byte offset within that line.
/// If cursor is on a '\n' byte, it renders at the end of the current line.
fn cursor_line_col(input: &str, pos: usize) -> (usize, usize) {
    let clamped = pos.min(input.len());
    let mut line_start = 0;
    let mut line = 0;
    for (nl_idx, _) in input.match_indices('\n') {
        if clamped <= nl_idx {
            return (line, clamped - line_start);
        }
        line_start = nl_idx + 1;
        line += 1;
    }
    (line, clamped - line_start)
}
