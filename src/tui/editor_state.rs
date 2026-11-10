//! Editor state — input buffer, cursor, autocomplete, and history navigation.
//!
//! Tracks `input_buffer`, `cursor_pos`, `autocomplete_mode`, `discovered_files`,
//! and history navigation state (`history_index`, `saved_input`, `saved_cursor`).

use super::editor::FileMatch;
use super::mode::AutocompleteMode;
use crate::image::ImageContent;

/// A pending image attached to the input buffer via an `[img:N]` token.
///
/// The `id` is the monotonic id embedded in the token; the `name` is a display
/// label (clipboard → `clipboard`, dropped file → file name). The image is
/// attached to the outgoing user message at turn start, then cleared.
#[derive(Debug, Clone)]
pub struct PendingImage {
    /// Monotonic id (the `N` in `[img:N]`).
    pub id: u32,
    /// Display label for the summary line and system messages.
    pub name: String,
    /// The normalized image content.
    pub image: ImageContent,
}

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
    /// Images attached to the input buffer via `[img:N]` tokens. The side
    /// table is keyed by the token's monotonic id; cleared at turn start.
    pub pending_images: Vec<PendingImage>,
    /// Next monotonic id to assign to a new pending image.
    pub next_image_id: u32,
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
            pending_images: Vec::new(),
            next_image_id: 0,
        }
    }

    /// Attach a new pending image, returning the id to embed in the buffer as
    /// `[img:{id}]`.
    pub fn add_pending_image(&mut self, name: String, image: ImageContent) -> u32 {
        let id = self.next_image_id;
        self.next_image_id += 1;
        self.pending_images.push(PendingImage { id, name, image });
        id
    }

    /// Remove a pending image by id (when its token is deleted from the buffer).
    pub fn remove_pending_image(&mut self, id: u32) {
        self.pending_images.retain(|p| p.id != id);
    }

    /// Clear all pending images (at turn start, after attaching to the message).
    pub fn clear_pending_images(&mut self) {
        self.pending_images.clear();
    }
}

impl Default for EditorState {
    fn default() -> Self {
        Self::new()
    }
}
