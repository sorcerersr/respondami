//! Editor state — input buffer, cursor, autocomplete, and history navigation.
//!
//! Tracks `input_buffer`, `cursor_pos`, `autocomplete_mode`, `discovered_files`,
//! and history navigation state (`history_index`, `saved_input`, `saved_cursor`).

use super::editor::FileMatch;
use super::mode::AutocompleteMode;

/// Editor-related state: input buffer, cursor, autocomplete.
#[derive(Debug)]
pub struct EditorState {
    pub input_buffer: String,
    pub cursor_pos: usize,
    pub autocomplete_mode: AutocompleteMode,
    pub discovered_files: Vec<FileMatch>,
    /// History navigation index. 0 = current input, 1+ = history entry.
    pub history_index: usize,
    /// Saved input buffer before entering history (set on first Up into history).
    pub saved_input: Option<String>,
    /// Saved cursor position before entering history.
    pub saved_cursor: usize,
}

impl EditorState {
    #[must_use]
    pub fn new() -> Self {
        Self {
            input_buffer: String::new(),
            cursor_pos: 0,
            autocomplete_mode: AutocompleteMode::None,
            discovered_files: Vec::new(),
            history_index: 0,
            saved_input: None,
            saved_cursor: 0,
        }
    }
}

impl Default for EditorState {
    fn default() -> Self {
        Self::new()
    }
}
