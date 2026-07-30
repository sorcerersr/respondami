//! Token statistics dialog key handler.
//!
//! Handles Esc to dismiss the dialog and return to Idle state.

use async_trait::async_trait;
use crossterm::event::KeyEvent;
use ratatui::backend::CrosstermBackend;
use ratatui::Terminal;

use crate::tui::{App, AppState};

use super::KeyHandler;

/// Handler for the token statistics dialog state.
#[derive(Debug)]
pub struct TokenStatsHandler;

#[async_trait(?Send)]
impl KeyHandler for TokenStatsHandler {
    async fn handle(
        &self,
        app: &mut App,
        key: &KeyEvent,
        _terminal: &mut Terminal<CrosstermBackend<std::io::Stdout>>,
    ) -> anyhow::Result<bool> {
        if key.code == crossterm::event::KeyCode::Esc {
            app.modal.state = AppState::Idle;
            app.modal.token_stats = None;
        }
        Ok(false)
    }
}
