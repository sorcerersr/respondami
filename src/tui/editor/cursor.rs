//! Editor cursor movement — character and visual-line navigation.
//!
//! Provides `cursor_left/right/up/down/home/end/backspace/delete` with proper
//! UTF-8 multi-byte character handling. Visual-line functions (`cursor_up_visual`,
//! `cursor_down_visual`) are soft-wrap aware, using display-width calculations.

use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

/// Find the last valid char-boundary byte index that is <= `pos`.
/// Only includes characters that fully fit within [0..=pos].
/// Returns 0 if no full character fits before or at `pos`.
///
/// Does NOT slice at `pos`, so it works even when `pos` is mid-character.
fn last_char_boundary_at_or_before(input: &str, pos: usize) -> usize {
    input
        .char_indices()
        .take_while(|&(idx, ch)| idx + ch.len_utf8() <= pos)
        .last()
        .map_or(0, |(idx, ch)| idx + ch.len_utf8())
}

/// Find the byte offset and width of the character immediately before `pos`.
/// Returns `None` if `pos == 0` (no character before cursor).
///
/// If `pos` falls inside a multi-byte character, rounds down to the previous
/// char boundary first (defensive — cursor should always be on char boundaries).
fn prev_char_offset(input: &str, pos: usize) -> Option<(usize, usize)> {
    if pos == 0 {
        return None;
    }
    let safe_pos = last_char_boundary_at_or_before(input, pos);
    if safe_pos == 0 {
        return None;
    }
    let before = &input[..safe_pos];
    let ch = before.chars().next_back().unwrap();
    let width = ch.len_utf8();
    Some((safe_pos - width, width))
}

/// Get the byte width of the character at byte offset `pos`.
///
/// If `pos` falls inside a multi-byte character, rounds down to the previous
/// char boundary (defensive — cursor should always be on char boundaries).
fn char_width_at(input: &str, pos: usize) -> usize {
    if pos >= input.len() {
        return 0;
    }
    let safe_pos = last_char_boundary_at_or_before(input, pos);
    if safe_pos >= input.len() {
        return 0;
    }
    input[safe_pos..].chars().next().map_or(0, char::len_utf8)
}

/// Convert a byte-offset cursor position to (`line_index`, `column_byte_offset`).
/// Lines are split on '\n'. The column is the byte offset within that line.
/// If cursor is on a '\n' byte, it renders at the end of the current line.
#[must_use]
pub fn cursor_line_col(input: &str, pos: usize) -> (usize, usize) {
    let clamped = pos.min(input.len());
    let mut line_start = 0;
    let mut line = 0;
    for (nl_idx, _) in input.match_indices('\n') {
        if clamped <= nl_idx {
            return (line, clamped - line_start);
        }
        line_start = nl_idx + 1; // skip past '\n'
        line += 1;
    }
    // Last line (or only line)
    (line, clamped - line_start)
}

/// Move cursor left by one character. Returns new position.
#[must_use]
pub fn cursor_left(input: &str, pos: usize) -> usize {
    if pos == 0 {
        return 0;
    }
    match prev_char_offset(input, pos) {
        Some((offset, width)) => {
            // If the previous character is '\n', land on the '\n' itself
            // (renderer shows cursor at end of previous line for '\n' positions)
            if width == 1 && input.as_bytes()[offset] == b'\n' {
                return offset;
            }
            offset
        }
        None => 0, // No character before cursor (mid-char or at start)
    }
}

/// Move cursor right by one character. Returns new position.
#[must_use]
pub fn cursor_right(input: &str, pos: usize) -> usize {
    if pos >= input.len() {
        return pos;
    }
    let width = char_width_at(input, pos);
    // If current char is '\n', skip to start of next line
    if width == 1 && input.as_bytes()[pos] == b'\n' {
        return pos + 1;
    }
    pos + width
}

/// Move cursor up one line. Returns new position.
#[must_use]
pub fn cursor_up(input: &str, pos: usize) -> usize {
    let (line, col) = cursor_line_col(input, pos);
    if line == 0 {
        return pos;
    }
    // Find the start of the previous line
    let lines: Vec<&str> = input.split('\n').collect();
    let prev_line = lines[line - 1];
    // Clamp by byte length first, then round down to nearest char boundary.
    // This prevents landing inside a multi-byte character when lines have
    // different UTF-8 layouts (e.g. ASCII line → line with wide chars).
    let clamped_col = last_char_boundary_at_or_before(prev_line, col.min(prev_line.len()));
    // Calculate byte offset to start of previous line
    let mut byte_offset = 0;
    for line_text in lines.iter().take(line - 1) {
        byte_offset += line_text.len() + 1; // +1 for '\n'
    }
    byte_offset + clamped_col
}

/// Move cursor down one line. Returns new position.
#[must_use]
pub fn cursor_down(input: &str, pos: usize) -> usize {
    let (line, col) = cursor_line_col(input, pos);
    let lines: Vec<&str> = input.split('\n').collect();
    if line >= lines.len() - 1 {
        return pos;
    }
    let next_line = lines[line + 1];
    // Clamp by byte length first, then round down to nearest char boundary.
    // This prevents landing inside a multi-byte character when lines have
    // different UTF-8 layouts (e.g. ASCII line → line with wide chars).
    let clamped_col = last_char_boundary_at_or_before(next_line, col.min(next_line.len()));
    // Calculate byte offset to start of next line
    let mut byte_offset = 0;
    for line_text in lines.iter().take(line + 1) {
        byte_offset += line_text.len() + 1; // +1 for '\n'
    }
    byte_offset + clamped_col
}

/// Move cursor to start of current line. Returns new position.
#[must_use]
pub fn cursor_home(input: &str, pos: usize) -> usize {
    let (line, _col) = cursor_line_col(input, pos);
    if line == 0 {
        return 0;
    }
    let lines: Vec<&str> = input.split('\n').collect();
    let mut byte_offset = 0;
    for line_text in lines.iter().take(line) {
        byte_offset += line_text.len() + 1;
    }
    byte_offset
}

/// Move cursor to end of current line. Returns new position.
#[must_use]
pub fn cursor_end(input: &str, pos: usize) -> usize {
    let (line, _col) = cursor_line_col(input, pos);
    let lines: Vec<&str> = input.split('\n').collect();
    if line >= lines.len() - 1 {
        return input.len();
    }
    let line_len = lines[line].len();
    let mut byte_offset = 0;
    for line_text in lines.iter().take(line) {
        byte_offset += line_text.len() + 1;
    }
    byte_offset + line_len
}

/// Delete the character before the cursor. Handles line joining at line start.
///
/// # Panics
///
/// - If `prev_char_offset` returns `None` for a non-empty string with cursor > 0.
pub fn cursor_backspace(input: &mut String, pos: &mut usize) {
    if *pos == 0 || input.is_empty() {
        return;
    }
    // If cursor is on a '\n' (e.g. after cursor_left from start of next line),
    // remove that '\n' to join lines
    if *pos < input.len() && input.as_bytes()[*pos] == b'\n' {
        input.remove(*pos);
        *pos -= 1;
    } else {
        let (offset, width) = prev_char_offset(input, *pos).unwrap();
        input.remove(offset);
        *pos -= width;
    }
}

/// Delete the character at the cursor. Handles line joining on '\n'.
pub fn cursor_delete(input: &mut String, pos: &mut usize) {
    if *pos >= input.len() {
        return;
    }
    let width = char_width_at(input, *pos);
    input.remove(*pos);
    // If we deleted '\n', cursor stays (lines joined)
    // Otherwise, cursor stays (next char shifts into position)
    let _ = width;
}

// ---------------------------------------------------------------------------
// Visual-line cursor movement (soft-wrap aware)
// ---------------------------------------------------------------------------

/// Convert a display-width column to a byte offset within `text`.
///
/// Walks characters summing display width until reaching `target_width`.
/// Returns the byte offset where the cursor should land.
/// If `target_width` exceeds total display width, returns `text.len()`.
#[must_use]
pub fn display_width_to_byte_offset(text: &str, target_width: usize) -> usize {
    if text.is_empty() {
        return 0;
    }
    let mut acc = 0;
    for (idx, ch) in text.char_indices() {
        let ch_width = ch.width().unwrap_or(0);
        if acc + ch_width > target_width {
            return idx;
        }
        acc += ch_width;
    }
    text.len()
}

/// Move cursor up one **visual** line (soft-wrap aware). Returns new position.
///
/// Uses `wrap_width` to determine soft-wrap boundaries. If the cursor is already
/// on the first visual line, returns the current position unchanged (caller
/// handles history navigation).
#[must_use]
pub fn cursor_up_visual(input: &str, pos: usize, wrap_width: usize) -> usize {
    if wrap_width == 0 || input.is_empty() {
        return pos;
    }

    let (visual_line, visual_col) = super::wrap::cursor_visual_pos(input, pos, wrap_width);
    if visual_line == 0 {
        return pos; // At top visual line
    }

    let visual_lines = super::wrap::build_visual_lines(input, wrap_width);
    let target = visual_lines[visual_line - 1];
    let (_logical_line, seg_start, seg_end) = target;

    // Get the text of the target logical line
    let line_text = input.split('\n').nth(_logical_line).unwrap_or("");
    let segment_text = &line_text[seg_start..seg_end];
    let segment_width = segment_text.width();

    // Clamp visual column to segment width
    let clamped_col = visual_col.min(segment_width);

    // Convert display-width column to byte offset within segment
    let byte_in_segment = display_width_to_byte_offset(segment_text, clamped_col);

    // Calculate byte offset to start of target logical line
    let mut byte_offset = 0;
    for line_text in input.split('\n').take(_logical_line) {
        byte_offset += line_text.len() + 1; // +1 for '\n'
    }

    byte_offset + seg_start + byte_in_segment
}

/// Move cursor down one **visual** line (soft-wrap aware). Returns new position.
///
/// Uses `wrap_width` to determine soft-wrap boundaries. If the cursor is already
/// on the last visual line, returns the current position unchanged (caller
/// handles history navigation).
#[must_use]
pub fn cursor_down_visual(input: &str, pos: usize, wrap_width: usize) -> usize {
    if wrap_width == 0 || input.is_empty() {
        return pos;
    }

    let (visual_line, visual_col) = super::wrap::cursor_visual_pos(input, pos, wrap_width);
    let visual_lines = super::wrap::build_visual_lines(input, wrap_width);

    if visual_line >= visual_lines.len().saturating_sub(1) {
        return pos; // At bottom visual line
    }

    let target = visual_lines[visual_line + 1];
    let (_logical_line, seg_start, seg_end) = target;

    // Get the text of the target logical line
    let line_text = input.split('\n').nth(_logical_line).unwrap_or("");
    let segment_text = &line_text[seg_start..seg_end];
    let segment_width = segment_text.width();

    // Clamp visual column to segment width
    let clamped_col = visual_col.min(segment_width);

    // Convert display-width column to byte offset within segment
    let byte_in_segment = display_width_to_byte_offset(segment_text, clamped_col);

    // Calculate byte offset to start of target logical line
    let mut byte_offset = 0;
    for line_text in input.split('\n').take(_logical_line) {
        byte_offset += line_text.len() + 1; // +1 for '\n'
    }

    byte_offset + seg_start + byte_in_segment
}
