use std::time::Instant;

use super::mode::{AppState, PopupType};
use super::token_stats::ProjectTokenStats;
use crate::tui::editor::PaletteCommand;
use crate::session::SessionMeta;

/// Modal and popup state: current app state, session selection, command palette, popups.
#[derive(Debug)]
pub struct ModalState {
    pub state: AppState,
    pub popup_selection: usize,
    pub session_select_matches: Vec<SessionMeta>,
    pub session_select_index: usize,
    pub popup_animation: Option<(PopupType, Instant)>,
    pub command_palette_query: String,
    pub command_palette_matches: Vec<(usize, PaletteCommand)>,
    pub command_palette_selected: usize,
    pub command_palette_scroll_offset: usize,
    pub command_palette_preserved_input: String,
    pub command_palette_preserved_cursor: usize,
    pub compact_keep_input: String,
    /// Token stats result. `None` = loading; `Some(Ok(stats))` = stats (possibly
    /// empty when there are no sessions); `Some(Err(msg))` = a read error to show.
    pub token_stats: Option<Result<ProjectTokenStats, String>>,
}

impl ModalState {
    #[must_use]
    pub fn new() -> Self {
        Self {
            state: AppState::Idle,
            popup_selection: 0,
            session_select_matches: Vec::new(),
            session_select_index: 0,
            popup_animation: None,
            command_palette_query: String::new(),
            command_palette_matches: Vec::new(),
            command_palette_selected: 0,
            command_palette_scroll_offset: 0,
            command_palette_preserved_input: String::new(),
            command_palette_preserved_cursor: 0,
            compact_keep_input: String::new(),
            token_stats: None,
        }
    }
}

impl Default for ModalState {
    fn default() -> Self {
        Self::new()
    }
}
