//! Modal layer — global shortcuts that are blocked when a modal is open.
//!
//! Modal states: `SessionSelect`, `InitPopup`, `CommandPalette`, `HelpPopup`
//!
//! Blocked shortcuts when modal is open:
//! - PgUp/PgDown (scroll chat)
//!
//! Always available (even in modals):
//! - Ctrl+D (quit)
//!
//! Delegated to `streaming_ui::handle_ui_shortcuts()`:
//! - F1 (open help)
//! - Ctrl+O/Ctrl+/ (toggle reasoning display)
//! - Ctrl+T (toggle tool output)
//!
//! HelpPopup-specific:
//! - Esc dismisses the help popup

use crossterm::event::KeyEvent;
use crate::tui::App;
use crate::tui::AppState;
use super::super::KeyEventResult;

/// Modal layer that handles global shortcuts with modal awareness.
#[derive(Debug, Default)]
pub struct ModalLayer;

impl ModalLayer {
    /// Create a new `ModalLayer` (no configuration needed).
    #[must_use]
    pub fn new() -> Self {
        Self
    }

    /// Handle a key event. Returns `KeyEventResult` indicating the outcome.
    pub fn handle(&self, app: &mut App, key: &KeyEvent) -> anyhow::Result<KeyEventResult> {
        // Ctrl+D — always quit, even in modals
        if key.code == crossterm::event::KeyCode::Char('d')
            && key.modifiers.contains(crossterm::event::KeyModifiers::CONTROL)
        {
            return Ok(KeyEventResult::Quit);
        }

        // If a modal is open, block the following shortcuts
        if self.is_modal_open(app) {
            return Ok(KeyEventResult::Unhandled);
        }

        // F1, Ctrl+O, Ctrl+/ , Ctrl+T — delegated to shared UI shortcuts
        if super::streaming_ui::handle_ui_shortcuts(app, key)? {
            return Ok(KeyEventResult::Handled);
        }

        // PgUp/PgDown — scroll chat
        match key.code {
            crossterm::event::KeyCode::PageUp => {
                let page = crate::agent_events::get_chat_visible_height(app);
                app.chat.scroll_up(page);
                return Ok(KeyEventResult::Handled);
            }
            crossterm::event::KeyCode::PageDown => {
                let page = crate::agent_events::get_chat_visible_height(app);
                app.chat.scroll_down(page);
                return Ok(KeyEventResult::Handled);
            }
            _ => {}
        }

        Ok(KeyEventResult::Unhandled)
    }

    /// Check if any modal is currently open.
    fn is_modal_open(&self, app: &App) -> bool {
        matches!(
            app.modal.state,
            AppState::SessionSelect
                | AppState::InitPopup
                | AppState::CommandPalette
                | AppState::HelpPopup
        )
    }
}
