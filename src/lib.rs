//! Respondami — a terminal-based AI coding agent.
//!
//! Provides the main application loop: terminal setup, event dispatch, TUI rendering,
//! animation ticks, compaction task polling, and graceful shutdown. Spawns the agent
//! loop via `tokio::spawn` and communicates through `AgentEvent` channels.

pub mod agent;
pub mod agent_events;
pub mod agents_md;
pub mod commands;
pub mod config;
pub mod context;
pub mod event_loop;
pub mod hooks;
pub mod key_handler;
pub mod logging;
pub mod mouse;
pub mod provider;
pub mod session;
pub mod skills;
pub mod sse_debug;
pub mod tools;
pub mod turn;
pub mod tui;

#[cfg(test)]
mod agent_events_tests;
#[cfg(test)]
mod agents_md_tests;
#[cfg(test)]
mod commands_tests;
#[cfg(test)]
mod config_tests;
#[cfg(test)]
mod event_loop_tests;
#[cfg(test)]
mod logging_tests;
#[cfg(test)]
mod sse_debug_tests;
#[cfg(test)]
mod skills_tests;

use std::path::PathBuf;
use std::time::Duration;

use crossterm::event::{Event, KeyEventKind, MouseEventKind, EventStream,
    KeyboardEnhancementFlags, PushKeyboardEnhancementFlags, PopKeyboardEnhancementFlags};
use futures_util::StreamExt;
use tokio::time::sleep;
use ratatui::backend::CrosstermBackend;
use ratatui::crossterm::execute;
use ratatui::crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen,
};
use ratatui::Terminal;

use crate::config::Config;
use crate::tui::{App, AppState, Theme};

/// Run the application.
///
/// # Errors
///
/// - Config load fails if the config file is malformed or invalid.
/// - Terminal setup fails if the terminal cannot be put into raw mode.
/// - Session loading fails if the session directory is unreadable.
///
/// # Panics
///
/// - If the compaction task handle is unexpectedly `None` during shutdown cleanup.
pub async fn run() -> anyhow::Result<()> {
    // Initialize file-based logging (.respondami/logs/respondami.log)
    logging::init();
    tracing::info!("respondami starting");

    // Initialize config
    let config = Config::load()?;
    tracing::info!("Config loaded");

    // Check endpoint (non-blocking, result shown as system message via oneshot)
    let provider = crate::provider::Provider::from_settings(&config.provider.settings);
    let (endpoint_tx, mut endpoint_rx) = tokio::sync::oneshot::channel();
    let endpoint_check = tokio::spawn(async move {
        let result = match provider {
            Ok(p) => match p.ping().await {
                Ok(()) => Ok(()),
                Err(e) => Err(format!("LLM endpoint unreachable: {e}")),
            },
            Err(e) => Err(format!("Provider not configured: {e}")),
        };
        if let Err(ref msg) = result {
            tracing::warn!("{}", msg);
        }
        let _ = endpoint_tx.send(result);
    });

    // Get CWD
    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));

    // Setup terminal — enter raw mode first to disable echo and canonical mode.
    crossterm::terminal::enable_raw_mode()?;
    execute!(std::io::stdout(), EnterAlternateScreen,
        crossterm::style::SetBackgroundColor(crossterm::style::Color::Rgb { r: 0x15, g: 0x1B, b: 0x23 }),
    )?;
    // Enable bracketed paste — terminal sends paste as structured Event::Paste.
    execute!(std::io::stdout(), crossterm::event::EnableBracketedPaste)?;
    // Enable Kitty keyboard protocol for modifier detection (Shift+Enter, etc.)
    // Gracefully degrades on unsupported terminals (sequence is silently ignored).
    execute!(
        std::io::stdout(),
        PushKeyboardEnhancementFlags(KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES)
    )?;
    // Enable button-event mouse tracking (?1002h) for scroll wheel support.
    // Shift+drag bypasses capture for native terminal text selection.
    execute!(std::io::stdout(), crate::mouse::EnableMouseScroll)?;
    let backend = CrosstermBackend::new(std::io::stdout());
    let mut terminal = Terminal::new(backend)?;

    // Create app
    let mut app = App::new(config, cwd);
    let theme = Theme::gh_dark();

    // Discover files for @-autocomplete
    app.discover_files();

    // Show endpoint check result as a system message (non-blocking)
    if let Ok(Err(msg)) = endpoint_rx.try_recv() {
        app.add_system_message(&format!("⚠ {msg}"));
    }

    // Async event stream — proper tokio integration, no background threads.
    let mut events = EventStream::new();

    // Main event loop
    loop {
        // Draw
        event_loop::draw_frame(&mut app, &mut terminal, &theme)?;

        // Tick activity indicator animation (time-based, ~10 ticks/s)
        event_loop::tick_activity_indicator(&mut app, &theme);

        // Poll in-flight compaction task from manual compaction.
        if let Some(task) = app.compaction_task.as_mut()
            && task.is_finished()
        {
            let handle = app.compaction_task.take().unwrap();
            match event_loop::handle_compaction_result(&mut app, handle, false).await {
                event_loop::CompactionResult::Success { .. } => {}
                event_loop::CompactionResult::Failed(_)
                | event_loop::CompactionResult::Panicked => {}
            }
            app.modal.state = AppState::Idle;
        }

        // Wait for event or animation tick
        // When effects are running, use shorter timeout for smoother animation
        let tick_duration = if app.ui.effect_manager.is_running() {
            Duration::from_millis(16) // ~60fps for effects
        } else {
            Duration::from_millis(50)
        };
        let event = tokio::select! {
            maybe_event = events.next() => match maybe_event {
                Some(Ok(ev)) => Some(ev),
                Some(Err(_)) | None => None,
            },
            // Animation timeout — redraw for working indicator movement / effects
            () = sleep(tick_duration) => None,
        };

        if let Some(ev) = event {
            match ev {
                Event::Key(key) if key.kind == KeyEventKind::Press => {
                    let was_compacting = app.modal.state == AppState::Compacting;

                    if key_handler::handle_key_event(&mut app, &key, &mut terminal).await? {
                        // Quit
                        execute!(std::io::stdout(), crate::mouse::DisableMouseScroll)?;
                        execute!(std::io::stdout(), crossterm::event::DisableBracketedPaste)?;
                        execute!(std::io::stdout(), PopKeyboardEnhancementFlags)?;
                        execute!(std::io::stdout(), LeaveAlternateScreen,
                            crossterm::style::ResetColor,
                        )?;
                        let _ = crossterm::terminal::disable_raw_mode();
                        endpoint_check.await.ok();
                        tracing::info!("Quitting respondami");
                        return Ok(());
                    }

                    // If user cancelled during compaction, abort the background task.
                    if was_compacting && app.modal.state != AppState::Compacting
                        && let Some(task) = app.compaction_task.take()
                    {
                        task.abort();
                        app.add_system_message("Compaction cancelled.");
                        app.chat.auto_scroll = true;
                    }
                }
                Event::Paste(text) => {
                    let text: String = text.chars().filter(|c| !c.is_control() || *c == '\n').take(10_000).collect();
                    app.editor.input_buffer.insert_str(app.editor.cursor_pos, &text);
                    app.editor.cursor_pos += text.len();
                    app.chat.scroll_to_bottom();
                }
                Event::Mouse(mouse) => {
                    match mouse.kind {
                        MouseEventKind::ScrollUp => {
                            app.chat.scroll_up(crate::mouse::SCROLL_LINES);
                        }
                        MouseEventKind::ScrollDown => {
                            app.chat.scroll_down(crate::mouse::SCROLL_LINES);
                        }
                        _ => {} // clicks, drags, moves — ignored; Shift+drag = native selection
                    }
                }
                Event::Resize(_, height) => {
                    app.ui.terminal_height = height as usize;
                    terminal.clear()?;
                }
                _ => {}
            }
        }
    }
}
