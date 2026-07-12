//! TUI mode enums — application states, popup types, and autocomplete modes.
//!
//! Defines `AppState` (Idle, Streaming, `ToolExec`, Compacting, etc.),
//! `PopupType` (for shared expand animations), and `AutocompleteMode`
//! (File/Skill/None with selection state).

/// Popup types that use the shared expand animation.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum PopupType {
    Init,
    SessionSelect,
    FileAutocomplete,
    SkillAutocomplete,
    CommandPalette,
    Help,
}

/// Application states.
#[derive(Debug, Clone, PartialEq)]
pub enum AppState {
    Idle,
    Streaming,
    ToolExec,
    Compacting,
    SessionSelect,
    /// Popup asking user if they want to generate AGENTS.md.
    InitPopup,
    /// Command palette overlay (opened via Ctrl+G).
    CommandPalette,
    /// Help popup overlay (opened via F1 or command palette).
    HelpPopup,
}

use super::editor::FileMatch;

/// Autocomplete mode for the editor.
#[derive(Debug, Clone, PartialEq)]
pub enum AutocompleteMode {
    None,
    File {
        matches: Vec<FileMatch>,
        selected: usize,
        scroll_offset: usize,
        /// Whether hidden files (dotfiles, dotdirs) are shown.
        show_hidden: bool,
    },
    Skill {
        matches: Vec<String>,
        selected: usize,
        scroll_offset: usize,
    },
}


