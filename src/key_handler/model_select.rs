//! `ModelSelect` handler — composed from `NavigationLayer` and `StateTransitionLayer`.
//!
//! Handles:
//! - List navigation (Up/Down/j/k)
//! - Enter (switch to selected model), Esc (cancel)
//! - Ctrl+D (quit)

use async_trait::async_trait;
use crossterm::event::KeyEvent;
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;

use crate::tui::{App, AppState};

use super::KeyHandler;
use super::layers::{NavigationLayer, StateTransitionLayer, TransitionAction};

/// Switch to the selected model and update live state.
fn do_select_model(app: &mut App) {
    let Some(selected) = app
        .modal
        .model_select_models
        .get(app.modal.model_select_index)
    else {
        return;
    };
    let name = selected.clone();
    app.switch_model(&name);
}

/// Cancel the model select: return to Idle. Cancelling changes nothing, so no
/// system message is emitted (one would clobber the welcome screen and add no
/// information to an existing conversation).
pub(crate) fn do_cancel_model_select(app: &mut App) {
    app.modal.state = AppState::Idle;
}

/// Handler for the model select modal.
#[derive(Debug)]
pub struct ModelSelectHandler;

#[async_trait(?Send)]
impl KeyHandler for ModelSelectHandler {
    async fn handle(
        &self,
        app: &mut App,
        key: &KeyEvent,
        _terminal: &mut Terminal<CrosstermBackend<std::io::Stdout>>,
    ) -> anyhow::Result<bool> {
        // 1. State transitions
        let mut transitions = StateTransitionLayer::new()
            .with_enter(Box::new(|app| {
                do_select_model(app);
                app.modal.state = AppState::Idle;
                app.chat.auto_scroll = true;
                TransitionAction::None
            }))
            .with_esc(Box::new(|app| {
                do_cancel_model_select(app);
            }));

        if let Some(action) = transitions.handle(app, key) {
            match action {
                TransitionAction::Send => unreachable!("ModelSelect never sends"),
                TransitionAction::RunTurnWithInput(_) => {
                    unreachable!("ModelSelect never runs turn with input")
                }
                TransitionAction::None => return Ok(false),
            }
        }

        // 2. Navigation
        let mut navigation = NavigationLayer::new(
            Box::new(|app| app.modal.model_select_models.len()),
            Box::new(|app| app.modal.model_select_index),
            Box::new(|app, idx| {
                app.modal.model_select_index = idx;
            }),
        );

        if navigation.handle(app, key)? {
            return Ok(false);
        }

        Ok(false)
    }
}
