//! Text editor module for the prompt input area.
//!
//! Provides cursor movement, text wrapping, file discovery, command parsing,
//! and the `EditorRenderer` for rendering the input area.
//!
//! Rust guideline compliant 2026-02-21

mod cursor;
mod renderer;
mod discovery;
mod commands;
pub(crate) mod wrap;

#[doc(inline)]
pub use cursor::{
    cursor_line_col, cursor_left, cursor_right,
    cursor_up, cursor_down, cursor_home, cursor_end,
    cursor_backspace, cursor_delete,
    cursor_up_visual, cursor_down_visual, display_width_to_byte_offset,
};
#[doc(inline)]
pub use renderer::{EditorRenderer, FILE_POPUP_PAGE_SIZE};
#[doc(inline)]
pub use discovery::{FileDiscovery, FileMatch};
#[doc(inline)]
pub use commands::{
    parse_file_references,
    PaletteCommand, get_palette_commands, fuzzy_match_palette_commands,
    fuzzy_match_case_insensitive,
};
#[doc(inline)]
pub use wrap::{wrap_line, build_visual_lines, cursor_visual_pos};

#[cfg(test)]
mod commands_tests;
#[cfg(test)]
mod cursor_tests;
#[cfg(test)]
mod discovery_tests;
#[cfg(test)]
mod wrap_tests;
