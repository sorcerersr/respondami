//! `SessionSelect` handler — composed from `NavigationLayer` and `StateTransitionLayer`.
//!
//! Handles:
//! - List navigation (Up/Down/j/k)
//! - Enter (load selected session), Esc (cancel)
//! - Ctrl+D (quit)

use async_trait::async_trait;
use crossterm::event::KeyEvent;
use ratatui::backend::CrosstermBackend;
use ratatui::Terminal;

use crate::tui::{App, AppState};

use super::layers::{NavigationLayer, StateTransitionLayer, TransitionAction};
use super::KeyHandler;

/// Load the selected session and rebuild the chat display.
fn do_load_session(app: &mut App) -> anyhow::Result<()> {
    let selected = &app.modal.session_select_matches[app.modal.session_select_index];
    app.session.session_store.load_session(&selected.path)?;
    app.chat.chat_messages.clear();
    // Restore cumulative token usage from saved session data
    let (input_tokens, output_tokens) = app.session.session_store.total_usage();
    app.session.cumulative_usage = crate::session::RequestTokenUsage {
        input_tokens,
        output_tokens,
        estimated: false,
    };
    // Restore token rate tracker from saved entries
    let (tokens, seconds) = app.session.session_store.total_token_rate();
    app.restore_tracker(tokens, seconds);
    // Restore activated skills from session metadata
    let activated = app.session.session_store.get_activated_skills();
    app.active_skills = activated.into_iter().collect();
    // Rebuild chat display from session using the adapter
    let adapter = crate::session::SessionDisplayAdapter::new(app.config.tool_output_expanded);
    app.chat.chat_messages = adapter.build_messages(app.session.session_store.entries());
    app.modal.state = AppState::Idle;
    app.chat.auto_scroll = true;
    Ok(())
}

/// Handler for the session select modal.
#[derive(Debug)]
pub struct SessionSelectHandler;

#[async_trait(?Send)]
impl KeyHandler for SessionSelectHandler {
    async fn handle(
        &self,
        app: &mut App,
        key: &KeyEvent,
        _terminal: &mut Terminal<CrosstermBackend<std::io::Stdout>>,
    ) -> anyhow::Result<bool> {
        // 1. State transitions
        let mut transitions = StateTransitionLayer::new()
            .with_enter(Box::new(|app| {
                if let Err(e) = do_load_session(app) {
                    // Show error and stay in session select
                    tracing::error!("Failed to load session: {}", e);
                }
                TransitionAction::None
            }))
            .with_esc(Box::new(|app| {
                app.modal.state = AppState::Idle;
                app.add_system_message("Session selection cancelled.");
                app.chat.auto_scroll = true;
            }));

        if let Some(action) = transitions.handle(app, key) {
            match action {
                TransitionAction::Send => unreachable!("SessionSelect never sends"),
                TransitionAction::RunTurnWithInput(_) => unreachable!("SessionSelect never runs turn with input"),
                TransitionAction::None => return Ok(false),
            }
        }

        // 2. Navigation
        let mut navigation = NavigationLayer::new(
            Box::new(|app| app.modal.session_select_matches.len()),
            Box::new(|app| app.modal.session_select_index),
            Box::new(|app, idx| {
                app.modal.session_select_index = idx;
            }),
        );

        if navigation.handle(app, key)? {
            return Ok(false);
        }

        Ok(false)
    }
}
