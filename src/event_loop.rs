//! Shared event loop helpers.
//!
//! Extracts duplicated logic between `lib.rs` (main loop) and `agent_events.rs`
//! (agent event loop) — draw, animation tick, and compaction task polling.
//!
//! Rust guideline compliant 2026-02-21

use std::fmt;

use ratatui::backend::CrosstermBackend;
use ratatui::Terminal;

use crate::session::CompactionPlan;
use crate::tui::{App, AppState, Theme};

/// Result of polling a compaction task.
///
/// Returned by [`handle_compaction_result`] so callers can decide what to do
/// next (restart agent, transition to idle, set `done = true`, etc.).
#[derive(Debug)]
pub enum CompactionResult {
    /// Compaction succeeded. Contains token counts and removed message count.
    Success {
        tokens_before: u32,
        tokens_after: u32,
        messages_removed: u32,
    },
    /// Compaction failed (provider error, serialization error, etc.).
    Failed(String),
    /// Compaction task panicked (`JoinError`).
    Panicked,
}

impl fmt::Display for CompactionResult {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CompactionResult::Success { .. } => write!(f, "compaction succeeded"),
            CompactionResult::Failed(msg) => write!(f, "compaction failed: {msg}"),
            CompactionResult::Panicked => write!(f, "compaction task panicked"),
        }
    }
}

/// Draw a single TUI frame.
///
/// Updates `app.ui.terminal_height` from the frame area and renders the layout.
/// Shared between the main loop (`lib.rs`) and the agent event loop
/// (`agent_events.rs`) to eliminate duplicated `terminal.draw()` closures.
///
/// # Errors
///
/// Returns an error if the terminal draw operation fails.
pub fn draw_frame(
    app: &mut App,
    terminal: &mut Terminal<CrosstermBackend<std::io::Stdout>>,
    theme: &Theme,
) -> anyhow::Result<()> {
    terminal.draw(|frame| {
        let height = frame.area().height as usize;
        app.ui.terminal_height = height;
        crate::tui::LayoutRenderer::render(frame, app, theme);
    })?;
    Ok(())
}

/// Tick the activity indicator animation if the app is in a working state.
///
/// Uses time-based ticking (~100ms interval) so the animation rate is
/// consistent regardless of which loop drives it.
pub fn tick_activity_indicator(app: &mut App, theme: &Theme) {
    if app.is_working() {
        let (label, _, _) = crate::tui::status_bar::get_activity(&app.modal.state, theme);
        app.ui.activity_indicator.tick(label);
    }
}

/// Poll a finished compaction task and apply the result.
///
/// Handles the `JoinHandle` match, calls `apply_compaction`, and pushes the
/// appropriate system/compaction message. Returns a [`CompactionResult`] enum
/// so callers can branch on caller-specific logic (e.g., agent restart vs
/// idle transition).
///
/// When `pinned_scroll_guard` is `true`, `auto_scroll` is guarded by
/// `!app.chat.pinned_scroll`. When `false`, `auto_scroll` is set
/// unconditionally (correct for user-initiated compaction).
pub async fn handle_compaction_result(
    app: &mut App,
    handle: tokio::task::JoinHandle<anyhow::Result<CompactionPlan>>,
    pinned_scroll_guard: bool,
) -> CompactionResult {
    match handle.await {
        Ok(Ok(plan)) => {
            match app.session.session_store.apply_compaction(
                plan.summary,
                plan.cut_index,
                plan.tokens_before,
            ) {
                Ok((tb, ta, mr)) => {
                    app.add_compaction_message(tb.saturating_sub(ta), mr);
                    if pinned_scroll_guard {
                        if !app.chat.pinned_scroll {
                            app.chat.auto_scroll = true;
                        }
                    } else {
                        app.chat.auto_scroll = true;
                    }
                    CompactionResult::Success {
                        tokens_before: tb,
                        tokens_after: ta,
                        messages_removed: mr,
                    }
                }
                Err(e) => {
                    app.add_system_message(&format!("Compaction failed: {e}"));
                    if pinned_scroll_guard {
                        if !app.chat.pinned_scroll {
                            app.chat.auto_scroll = true;
                        }
                    } else {
                        app.chat.auto_scroll = true;
                    }
                    CompactionResult::Failed(format!("{e}"))
                }
            }
        }
        Ok(Err(e)) => {
            app.add_system_message(&format!("Compaction failed: {e}"));
            if pinned_scroll_guard {
                if !app.chat.pinned_scroll {
                    app.chat.auto_scroll = true;
                }
            } else {
                app.chat.auto_scroll = true;
            }
            CompactionResult::Failed(format!("{e}"))
        }
        Err(_) => {
            app.add_system_message("Compaction task panicked.");
            if pinned_scroll_guard {
                if !app.chat.pinned_scroll {
                    app.chat.auto_scroll = true;
                }
            } else {
                app.chat.auto_scroll = true;
            }
            CompactionResult::Panicked
        }
    }
}

/// Transition the app to idle state and reset the compaction task.
///
/// Common cleanup when compaction finishes with an error (or is cancelled).
pub fn transition_to_idle(app: &mut App) {
    app.modal.state = AppState::Idle;
}
