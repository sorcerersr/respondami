//! Modal layer — global shortcuts that are blocked when a modal is open.
//!
//! Modal states: `SessionSelect`, `InitPopup`, `CommandPalette`, `HelpPopup`
//!
//! Blocked shortcuts when modal is open:
//! - PgUp/PgDown (scroll chat)
//! - Ctrl+O/Ctrl+/ (toggle reasoning display)
//! - Ctrl+T (toggle tool output)
//! - F1 (open help)
//!
//! Always available (even in modals):
//! - Ctrl+D (quit)
//!
//! HelpPopup-specific:
//! - Esc dismisses the help popup

use crossterm::event::KeyEvent;
use crate::tui::App;
use crate::tui::AppState;

/// Modal layer that handles global shortcuts with modal awareness.
#[derive(Debug)]
pub struct ModalLayer;

impl ModalLayer {
    /// Create a new `ModalLayer` (no configuration needed).
    pub fn new() -> Self {
        Self
    }

    /// Handle a key event. Returns `true` if the event caused a quit.
    /// Returns `false` if the event was handled but no quit, or if the event was not handled.
    pub fn handle(&self, app: &mut App, key: &KeyEvent) -> anyhow::Result<bool> {
        // Ctrl+D — always quit, even in modals
        if key.code == crossterm::event::KeyCode::Char('d')
            && key.modifiers.contains(crossterm::event::KeyModifiers::CONTROL)
        {
            return Ok(true);
        }

        // If a modal is open, block the following shortcuts
        if self.is_modal_open(app) {
            return Ok(false);
        }

        // F1 — open help popup
        if key.code == crossterm::event::KeyCode::F(1) {
            app.modal.state = AppState::HelpPopup;
            return Ok(false);
        }

        // Ctrl+O or Ctrl+/: toggle reasoning visibility
        let is_reasoning_toggle = (key.code == crossterm::event::KeyCode::Char('o')
            || key.code == crossterm::event::KeyCode::Char('/'))
            && key.modifiers.contains(crossterm::event::KeyModifiers::CONTROL);
        if is_reasoning_toggle {
            app.config.thinking_display = app.config.thinking_display.toggle();
            app.chat.auto_scroll = true;
            app.save_config()?;
            return Ok(false);
        }

        // Ctrl+T: toggle all tool call output expand/collapse
        let is_expand_toggle = key.code == crossterm::event::KeyCode::Char('t')
            && key.modifiers.contains(crossterm::event::KeyModifiers::CONTROL);
        if is_expand_toggle {
            app.toggle_all_tool_output();
            app.save_config()?;
            return Ok(false);
        }

        // PgUp/PgDown — scroll chat
        match key.code {
            crossterm::event::KeyCode::PageUp => {
                let page = crate::agent_events::get_chat_visible_height(app);
                app.chat.scroll_up(page);
                return Ok(false);
            }
            crossterm::event::KeyCode::PageDown => {
                let page = crate::agent_events::get_chat_visible_height(app);
                app.chat.scroll_down(page);
                return Ok(false);
            }
            _ => {}
        }

        Ok(false)
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
