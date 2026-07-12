//! `CommandPalette` handler — composed from `NavigationLayer` and `StateTransitionLayer`.
//!
//! Handles:
//! - List navigation (Up/Down/j/k/Tab/Shift+Tab)
//! - Enter (execute selected command), Esc (dismiss, restore input)
//! - Backspace (remove from filter), Ctrl+C (clear query)
//! - Ctrl+D (quit)

use async_trait::async_trait;
use crossterm::event::KeyEvent;
use ratatui::backend::CrosstermBackend;
use ratatui::Terminal;

use crate::tui::editor::fuzzy_match_palette_commands;
use crate::tui::{App, AppState};

use super::layers::{NavigationLayer, StateTransitionLayer, TransitionAction};
use super::KeyHandler;

/// Max visible commands in the palette (matches `LayoutRenderer::PALETTE_MAX_VISIBLE`).
const PALETTE_PAGE_SIZE: usize = 10;

/// Update scroll offset to keep the selected item visible.
fn update_scroll(app: &mut App, total: usize) {
    let selected = app.modal.command_palette_selected;
    let mut offset = app.modal.command_palette_scroll_offset;
    if selected >= offset + PALETTE_PAGE_SIZE {
        offset = selected - PALETTE_PAGE_SIZE + 1;
    } else if selected < offset {
        offset = selected;
    }
    app.modal.command_palette_scroll_offset = offset.min(total.saturating_sub(PALETTE_PAGE_SIZE));
}

/// Handler for the command palette modal.
#[derive(Debug)]
pub struct CommandPaletteHandler;

#[async_trait(?Send)]
impl KeyHandler for CommandPaletteHandler {
    async fn handle(
        &self,
        app: &mut App,
        key: &KeyEvent,
        terminal: &mut Terminal<CrosstermBackend<std::io::Stdout>>,
    ) -> anyhow::Result<bool> {
        // Compute matches once — stored in ModalState for all closures to read
        app.modal.command_palette_matches =
            fuzzy_match_palette_commands(&app.modal.command_palette_query, 10);

        // 1. State transitions
        let mut transitions = StateTransitionLayer::new()
            .with_enter(Box::new(|app| {
                let matches = &app.modal.command_palette_matches;
                if matches.get(app.modal.command_palette_selected).is_some() {
                    // Restore input before executing
                    app.editor.input_buffer = app.modal.command_palette_preserved_input.clone();
                    app.editor.cursor_pos = app.modal.command_palette_preserved_cursor;
                    app.modal.state = AppState::Idle;
                    TransitionAction::Send
                } else {
                    // No match — just dismiss
                    app.editor.input_buffer = app.modal.command_palette_preserved_input.clone();
                    app.editor.cursor_pos = app.modal.command_palette_preserved_cursor;
                    app.modal.state = AppState::Idle;
                    TransitionAction::None
                }
            }))
            .with_esc(Box::new(|app| {
                app.editor.input_buffer = app.modal.command_palette_preserved_input.clone();
                app.editor.cursor_pos = app.modal.command_palette_preserved_cursor;
                app.modal.state = AppState::Idle;
            }))
            .with_ctrl_c(Box::new(|app| {
                app.modal.command_palette_query.clear();
                app.modal.command_palette_selected = 0;
                app.modal.command_palette_scroll_offset = 0;
            }));

        if let Some(action) = transitions.handle(app, key) {
            match action {
                TransitionAction::Send => {
                    if let Some((_, cmd)) = app.modal.command_palette_matches.get(app.modal.command_palette_selected) {
                        return crate::commands::execute_palette_command(app, cmd.id, terminal).await;
                    }
                    return Ok(false);
                }
                TransitionAction::RunTurnWithInput(_) => unreachable!("CommandPalette never runs turn with input"),
                TransitionAction::None => return Ok(false),
            }
        }

        // 2. Navigation
        let matches_len = app.modal.command_palette_matches.len();
        let mut navigation = NavigationLayer::new(
            Box::new(move |_| matches_len),
            Box::new(|app| app.modal.command_palette_selected),
            Box::new(|app, idx| {
                app.modal.command_palette_selected = idx;
            }),
        );

        if navigation.handle(app, key)? {
            update_scroll(app, matches_len);
            return Ok(false);
        }

        // 3. Input (filter text)
        match key.code {
            crossterm::event::KeyCode::Char(c) => {
                app.modal.command_palette_query.push(c);
                app.modal.command_palette_selected = 0;
                app.modal.command_palette_scroll_offset = 0;
            }
            crossterm::event::KeyCode::Backspace => {
                app.modal.command_palette_query.pop();
                app.modal.command_palette_selected = 0;
                app.modal.command_palette_scroll_offset = 0;
            }
            _ => {}
        }

        Ok(false)
    }
}
