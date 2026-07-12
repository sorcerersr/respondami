//! Key handler chain pattern with composable layers.
//!
//! Each application state composes layers instead of implementing logic from scratch:
//! - `InputLayer` — character insertion, cursor movement, editing
//! - `NavigationLayer` — Up/Down/j/k/Tab list navigation
//! - `StateTransitionLayer` — Enter, Esc, Ctrl+C state transitions
//! - `ModalLayer` — modal-aware global shortcuts
//!
//! The top-level `handle_key_event()` is a thin dispatcher that:
//! 1. Runs truly global shortcuts (Ctrl+D)
//! 2. Runs autocomplete handling (when active in appropriate states)
//! 3. Delegates to the state-specific handler via `StateHandler` enum

mod global;
mod idle;
mod init_popup;
mod session_select;
mod command_palette;
mod help_popup;
mod layers;

use async_trait::async_trait;
use crossterm::event::KeyEvent;
use ratatui::backend::CrosstermBackend;
use ratatui::Terminal;

use crate::tui::App;

/// Trait for handling key events in a specific application state.
///
/// Returns `true` if the application should quit, `false` otherwise.
#[async_trait(?Send)]
pub trait KeyHandler {
    async fn handle(
        &self,
        app: &mut App,
        key: &KeyEvent,
        terminal: &mut Terminal<CrosstermBackend<std::io::Stdout>>,
    ) -> anyhow::Result<bool>;
}

/// Enum mapping each `AppState` to its corresponding handler.
///
/// This avoids trait objects and keeps dispatch fast and simple.
#[derive(Debug)]
pub enum StateHandler {
    Idle,
    SessionSelect,
    InitPopup,
    CommandPalette,
    HelpPopup,
}

impl StateHandler {
    /// Dispatch to the appropriate handler.
    ///
    /// # Errors
    ///
    /// - Hook execution errors (exit code != 0).
    /// - Agent streaming errors during compaction.
    pub async fn handle(
        &self,
        app: &mut App,
        key: &KeyEvent,
        terminal: &mut Terminal<CrosstermBackend<std::io::Stdout>>,
    ) -> anyhow::Result<bool> {
        match self {
            StateHandler::Idle => idle::IdleHandler.handle(app, key, terminal).await,
            StateHandler::SessionSelect => session_select::SessionSelectHandler.handle(app, key, terminal).await,
            StateHandler::InitPopup => init_popup::InitPopupHandler.handle(app, key, terminal).await,
            StateHandler::CommandPalette => command_palette::CommandPaletteHandler.handle(app, key, terminal).await,
            StateHandler::HelpPopup => help_popup::HelpPopupHandler.handle(app, key, terminal).await,
        }
    }
}

/// Thin dispatcher that delegates to the appropriate handler chain.
///
/// # Errors
///
/// - Hook execution errors (exit code != 0).
/// - Agent streaming errors during compaction.
pub async fn handle_key_event(
    app: &mut App,
    key: &KeyEvent,
    terminal: &mut Terminal<CrosstermBackend<std::io::Stdout>>,
) -> anyhow::Result<bool> {
    // 1. Truly global shortcuts (Ctrl+D only)
    if global::handle_global_shortcuts(app, key, terminal).await? {
        return Ok(true);
    }

    // 2. Modal-aware global shortcuts (blocked when modal is open)
    let modal_layer = layers::ModalLayer::new();
    if modal_layer.handle(app, key)? {
        return Ok(true);
    }

    // 3. Autocomplete handling (only in Idle state)
    if handle_autocomplete(app, key)? {
        return Ok(false);
    }

    // 4. State-specific handler
    let handler = app.current_handler();
    handler.handle(app, key, terminal).await
}

/// Handle autocomplete when it's active.
/// Returns `true` if autocomplete handled the key and the event loop should return early.
///
/// Only active in Idle state.
fn handle_autocomplete(
    app: &mut App,
    key: &KeyEvent,
) -> anyhow::Result<bool> {
    use crate::tui::AutocompleteMode;

    // Clone data to avoid borrow conflict with mutable app reference.
    let skill_data = match &app.editor.autocomplete_mode {
        AutocompleteMode::Skill { matches, selected, scroll_offset } => {
            Some((matches.clone(), *selected, *scroll_offset))
        }
        _ => None,
    };
    if let Some((matches, selected, scroll_offset)) = skill_data {
        return crate::tui::autocomplete::handle_skill_autocomplete(
            app, key, &matches, &selected, &scroll_offset,
        );
    }

    // File autocomplete — only when non-empty.
    let file_data = match &app.editor.autocomplete_mode {
        AutocompleteMode::File { matches, selected, scroll_offset, show_hidden } if !matches.is_empty() => {
            Some((matches.clone(), *selected, *scroll_offset, *show_hidden))
        }
        _ => None,
    };
    if let Some((matches, selected, scroll_offset, show_hidden)) = file_data {
        return crate::tui::autocomplete::handle_file_autocomplete(
            app, key, &matches, &selected, &scroll_offset, &show_hidden,
        );
    }

    Ok(false)
}
