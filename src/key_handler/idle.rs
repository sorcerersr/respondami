//! Idle state key handler — composed from `InputLayer` and `StateTransitionLayer`.
//!
//! Handles:
//! - Character input with @ trigger
//! - / trigger → skill autocomplete (`AutocompleteMode::Skill`)
//! - Enter (send message), Shift+Enter / Alt+Enter (newline)
//! - Ctrl+G (command palette)
//! - Ctrl+C / Ctrl+K (clear input), Esc (clear input)
//! - Cursor movement (Left, Right, Up, Down, Home, End, Backspace, Delete)

use async_trait::async_trait;
use crossterm::event::KeyEvent;
use ratatui::backend::CrosstermBackend;
use ratatui::Terminal;

use crate::tui::{App, AppState, AutocompleteMode};

use super::layers::{InputLayer, InputConfig, StateTransitionLayer, TransitionAction};
use super::KeyHandler;

/// Handler for the idle state.
#[derive(Debug)]
pub struct IdleHandler;

#[async_trait(?Send)]
impl KeyHandler for IdleHandler {
    async fn handle(
        &self,
        app: &mut App,
        key: &KeyEvent,
        terminal: &mut Terminal<CrosstermBackend<std::io::Stdout>>,
    ) -> anyhow::Result<bool> {
        // 1. State transitions (highest priority)
        let mut transitions = StateTransitionLayer::new()
            .with_ctrl_g(Box::new(|app| {
                app.modal.command_palette_preserved_input = app.editor.input_buffer.clone();
                app.modal.command_palette_preserved_cursor = app.editor.cursor_pos;
                app.modal.command_palette_query.clear();
                app.modal.command_palette_selected = 0;
                app.modal.state = AppState::CommandPalette;
            }))
            .with_ctrl_c(Box::new(|app| {
                app.editor.input_buffer.clear();
                app.editor.cursor_pos = 0;
                app.editor.autocomplete_mode = AutocompleteMode::None;
                app.reset_history();
            }))
            .with_ctrl_k(Box::new(|app| {
                app.editor.input_buffer.clear();
                app.editor.cursor_pos = 0;
                app.editor.autocomplete_mode = AutocompleteMode::None;
                app.reset_history();
            }))
            .with_alt_enter(Box::new(|app| {
                app.editor.input_buffer.insert(app.editor.cursor_pos, '\n');
                app.editor.cursor_pos += 1;
                app.editor.autocomplete_mode = AutocompleteMode::None;
            }))
            .with_shift_enter(Box::new(|app| {
                app.editor.input_buffer.insert(app.editor.cursor_pos, '\n');
                app.editor.cursor_pos += 1;
                app.editor.autocomplete_mode = AutocompleteMode::None;
            }))
            .with_esc(Box::new(|app| {
                app.editor.input_buffer.clear();
                app.editor.cursor_pos = 0;
                app.editor.autocomplete_mode = AutocompleteMode::None;
                app.reset_history();
            }))
            .with_enter(Box::new(|app| {
                app.reset_history();
                TransitionAction::Send
            }));

        if let Some(action) = transitions.handle(app, key) {
            match action {
                TransitionAction::Send => return crate::turn::start_turn(app, terminal).await,
                TransitionAction::RunTurnWithInput(_) => unreachable!("Idle never runs turn with input"),
                TransitionAction::None => return Ok(false),
            }
        }

        // 2. Handle / trigger for skill autocomplete (before InputLayer)
        if key.code == crossterm::event::KeyCode::Char('/')
            && app.editor.input_buffer.is_empty()
            && !key.modifiers.contains(crossterm::event::KeyModifiers::CONTROL)
        {
            app.editor.input_buffer.push('/');
            app.editor.cursor_pos = 1;
            let skill_names: Vec<String> = app.config.skills
                .iter()
                .map(|s| s.name.clone())
                .collect();
            app.editor.autocomplete_mode = AutocompleteMode::Skill {
                matches: skill_names,
                selected: 0,
                scroll_offset: 0,
            };
            return Ok(false);
        }

        // 3. Input (character insertion, cursor movement, triggers)
        let mut input = InputLayer::new(InputConfig {
            allow_newlines: false, // Shift+Enter handled by transitions above
            enable_at_trigger: true,
        })
        .with_at_trigger(Box::new(|app| {
            app.discover_files();
            let query = crate::tui::autocomplete::get_query_after_char(&app.editor.input_buffer, '@');
            let show_hidden = app.config.config.ui.file_show_hidden;
            let matches = app.fuzzy_match_files(&query, show_hidden);
            tracing::debug!(
                discovered = app.editor.discovered_files.len(),
                query = ?query,
                matches = matches.len(),
                "@ trigger"
            );
            app.editor.autocomplete_mode = AutocompleteMode::File {
                matches,
                selected: 0,
                scroll_offset: 0,
                show_hidden,
            };
        }));

        input.handle(app, key)
    }
}
