//! `InitPopup` handler — composed from `NavigationLayer` and `StateTransitionLayer`.
//!
//! Handles:
//! - List navigation (Up/Down/j/k)
//! - Enter (confirm/cancel), Esc (dismiss)
//! - Ctrl+D (quit)

use async_trait::async_trait;
use crossterm::event::KeyEvent;
use ratatui::backend::CrosstermBackend;
use ratatui::Terminal;

use crate::tui::{App, AppState};

use super::layers::{NavigationLayer, StateTransitionLayer, TransitionAction};
use super::KeyHandler;

/// Handler for the init popup.
#[derive(Debug)]
pub struct InitPopupHandler;

#[async_trait(?Send)]
impl KeyHandler for InitPopupHandler {
    async fn handle(
        &self,
        app: &mut App,
        key: &KeyEvent,
        terminal: &mut Terminal<CrosstermBackend<std::io::Stdout>>,
    ) -> anyhow::Result<bool> {
        // 1. State transitions
        let mut transitions = StateTransitionLayer::new()
            .with_enter(Box::new(|app| {
                if app.modal.popup_selection == 0 {
                    // Yes — generate AGENTS.md
                    app.modal.state = AppState::Idle;
                    app.add_user_message(crate::agents_md::GENERATE_CHAT_MESSAGE);
                    TransitionAction::RunTurnWithInput(crate::agents_md::GENERATE_PROMPT.to_string())
                } else {
                    // No — dismiss
                    app.modal.state = AppState::Idle;
                    TransitionAction::None
                }
            }))
            .with_esc(Box::new(|app| {
                app.modal.state = AppState::Idle;
            }));

        if let Some(action) = transitions.handle(app, key) {
            match action {
                TransitionAction::RunTurnWithInput(prompt) => {
                    return crate::turn::run_turn_with_input(app, prompt, None, terminal).await;
                }
                TransitionAction::Send => unreachable!("InitPopup never sends"),
                TransitionAction::None => return Ok(false),
            }
        }

        // 2. Navigation
        let mut navigation = NavigationLayer::new(
            Box::new(|_| 2usize), // Always 2 options: Yes/No
            Box::new(|app| app.modal.popup_selection),
            Box::new(|app, idx| {
                app.modal.popup_selection = idx;
            }),
        );

        if navigation.handle(app, key)? {
            return Ok(false);
        }

        Ok(false)
    }
}
