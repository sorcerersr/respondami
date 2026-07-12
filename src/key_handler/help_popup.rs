//! `HelpPopup` handler — dismisses on Esc.
//!
//! Handles:
//! - Esc (dismiss, return to Idle)

use async_trait::async_trait;
use crossterm::event::KeyEvent;
use ratatui::backend::CrosstermBackend;
use ratatui::Terminal;

use crate::tui::{App, AppState};

use super::KeyHandler;

/// Handler for the help popup.
#[derive(Debug)]
pub struct HelpPopupHandler;

#[async_trait(?Send)]
impl KeyHandler for HelpPopupHandler {
    async fn handle(
        &self,
        app: &mut App,
        key: &KeyEvent,
        _terminal: &mut Terminal<CrosstermBackend<std::io::Stdout>>,
    ) -> anyhow::Result<bool> {
        if key.code == crossterm::event::KeyCode::Esc {
            app.modal.state = AppState::Idle;
            return Ok(false);
        }
        Ok(false)
    }
}
