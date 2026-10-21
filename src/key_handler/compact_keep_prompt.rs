//! Keep-N prompt key handler for manual compaction.
//!
//! Accepts digit input (0–9) into `compact_keep_input`; non-digits are ignored.
//! Enter parses N (empty/invalid → 1) and runs `execute_compact`; Esc returns
//! to Idle. The prompt is pre-filled with `1`, so Enter with no typing keeps
//! the old one-press behavior.

use async_trait::async_trait;
use crossterm::event::{KeyCode, KeyEvent};
use ratatui::backend::CrosstermBackend;
use ratatui::Terminal;

use crate::commands::execute_compact;
use crate::tui::{App, AppState};

use super::KeyHandler;

/// Handler for the keep-N compaction prompt state.
#[derive(Debug)]
pub struct CompactKeepPromptHandler;

#[async_trait(?Send)]
impl KeyHandler for CompactKeepPromptHandler {
    async fn handle(
        &self,
        app: &mut App,
        key: &KeyEvent,
        terminal: &mut Terminal<CrosstermBackend<std::io::Stdout>>,
    ) -> anyhow::Result<bool> {
        match key.code {
            KeyCode::Esc => {
                app.modal.state = AppState::Idle;
                app.modal.compact_keep_input.clear();
            }
            KeyCode::Enter => {
                let keep: Option<usize> = app.modal.compact_keep_input.trim().parse().ok();
                app.modal.compact_keep_input.clear();
                execute_compact(app, keep, terminal).await?;
            }
            KeyCode::Char(c) if c.is_ascii_digit() => {
                app.modal.compact_keep_input.push(c);
            }
            _ => {}
        }
        Ok(false)
    }
}
