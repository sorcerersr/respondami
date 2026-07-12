//! Command palette execution — handles slash commands and Ctrl+G menu actions.
//!
//! Maps palette command IDs to their handlers: session management (new, compact,
//! resume), display toggles (thinking, reasoning), model switching, and quit.

use ratatui::backend::CrosstermBackend;
use ratatui::Terminal;

use crate::config::Config;
use crate::session::SessionStore;
use crate::tui::{App, AppState};

use super::turn::run_turn_with_input;

/// Execute a command from the command palette (Ctrl+G).
///
/// Each palette command acts directly on the app state.
///
/// # Errors
///
/// - Session persistence errors (file I/O).
/// - Agent streaming errors during compaction.
pub async fn execute_palette_command(
    app: &mut App,
    cmd_id: &str,
    terminal: &mut Terminal<CrosstermBackend<std::io::Stdout>>,
) -> anyhow::Result<bool> {
    match cmd_id {
        "new" => {
            if app.session.session_store.has_active_session() {
                app.session.session_store = SessionStore::new(&app.config.cwd);
                app.chat.chat_messages.clear();
                app.active_skills.clear();
                app.reset_request_usage();
                app.session.cumulative_usage = crate::session::RequestTokenUsage::default();
                app.session.session_prompt_tokens = 0;
                app.session.session_completion_tokens = 0;
            }
        }
        "resume" => {
            app.modal.session_select_matches = app.list_sessions()?;
            if app.modal.session_select_matches.is_empty() {
                app.add_system_message("No sessions found. Send a message to create one.");
                app.chat.auto_scroll = true;
            } else {
                app.modal.session_select_index = 0;
                app.modal.state = AppState::SessionSelect;
            }
        }
        "quit" | "q" => {
            return Ok(true);
        }
        "compact" => {
            if app.session.session_store.has_active_session() {
                // Close command palette, show "Compacting..." message.
                // Spawn LLM summarization as background task — UI redraws with sweep animation.
                app.modal.state = AppState::Compacting;
                app.add_system_message("Compacting session context...");
                app.chat.auto_scroll = true;

                let entries = app.session.session_store.entries().to_vec();
                let engine = crate::session::CompactionEngine::from_config(&app.config.config);
                let provider = crate::agent::build_provider(&app.config.config)?;
                let config = app.config.config.clone();
                let cwd = app.config.cwd.clone();
                let skills = app.config.skills.clone();

                app.compaction_task = Some(tokio::spawn(async move {
                    engine.compute_compaction(provider, config, entries, cwd, skills).await
                }));
                return Ok(false); // Return immediately — main loop redraws with animation
            }
            app.add_system_message("No active session to compact.");
            app.chat.auto_scroll = true;
        }
        "help" | "h" => {
            app.modal.state = AppState::HelpPopup;
        }
        "model" => {
            app.add_system_message(&format!(
                "Model: {}\nContext window: {} tokens\nContext usage: {}% ({} input tokens)\nSession totals: ↑{} input ↓{} output",
                app.config.model_name,
                app.config.context_window,
                if app.config.context_window > 0 {
                    (f64::from(app.session.current_request_usage.input_tokens) / f64::from(app.config.context_window) * 100.0) as u64
                } else {
                    0
                },
                app.session.current_request_usage.input_tokens,
                app.session.cumulative_usage.input_tokens,
                app.session.cumulative_usage.output_tokens,
            ));
            app.chat.auto_scroll = true;
        }
        "clear" => {
            app.clear_chat();
        }
        "init" => {
            app.add_user_message(crate::agents_md::GENERATE_CHAT_MESSAGE);
            let prompt = crate::agents_md::GENERATE_PROMPT.to_string();
            return run_turn_with_input(app, prompt, None, terminal).await;
        }
        // Toggle commands — act directly and persist
        "toggle_thinking" => {
            app.config.thinking_display = app.config.thinking_display.toggle();
            app.chat.auto_scroll = true;
            app.save_config()?;
        }
        "toggle_tool_output" => {
            app.toggle_all_tool_output();
            app.save_config()?;
        }
        "toggle_hook_mode" => {
            app.config.hook_display = app.config.hook_display.toggle();
            app.save_config()?;
        }
        "reload_hooks" => {
            app.config.hook_registry = crate::hooks::load_all_hooks(&Config::config_dir(), &app.config.cwd);
            app.add_system_message(&format!(
                "Hooks reloaded: {} active",
                app.config.hook_registry.total_count()
            ));
            app.chat.auto_scroll = true;
        }
        _ => {
            app.add_system_message(&format!("Unknown command: {cmd_id}"));
            app.chat.auto_scroll = true;
        }
    }
    Ok(false)
}

/// Get the description for a palette command, including dynamic state info for toggles.
///
/// For toggle commands, appends `[current → next]` to the description.
#[must_use]
pub fn get_palette_command_description(id: &str, app: &App) -> String {
    match id {
        "toggle_thinking" => {
            format!("Toggle thinking display {}", app.config.thinking_display.palette_mode_display())
        }
        "toggle_tool_output" => {
            let current = if app.config.tool_output_expanded { "expanded" } else { "collapsed" };
            let next = if app.config.tool_output_expanded { "collapsed" } else { "expanded" };
            format!("Toggle tool output [{current} → {next}]")
        }
        "toggle_hook_mode" => {
            format!("Toggle hook display {}", app.config.hook_display.palette_mode_display())
        }
        _ => {
            // Static description from PaletteCommand
            crate::tui::editor::get_palette_commands()
                .iter()
                .find(|c| c.id == id)
                .map(|c| c.description.to_string())
                .unwrap_or_default()
        }
    }
}

