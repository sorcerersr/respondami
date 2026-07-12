//! Global shortcuts that are always available regardless of application state.
//!
//! Only Ctrl+D (quit) is truly global — it works in all states, including modals.
//! All other global shortcuts (PgUp/PgDown, Ctrl+O, Ctrl+T) are now handled by
//! `ModalLayer` which blocks them when a modal is open.
//!
//! The dead Ctrl+G path has been removed — Ctrl+G is handled by the `IdleHandler`
//! (opening command palette) and the `ModalLayer` blocks it in non-Idle states.

use crossterm::event::KeyEvent;
use ratatui::backend::CrosstermBackend;
use ratatui::Terminal;

use crate::tui::App;

/// Handle global shortcuts. Returns `true` if the event caused a quit, `false` otherwise.
///
/// This is the only truly global handler — it runs before state-specific handlers
/// and only handles Ctrl+D (quit). All other shortcuts are handled by `ModalLayer`
/// which blocks them when a modal is open.
pub async fn handle_global_shortcuts(
    _app: &mut App,
    key: &KeyEvent,
    _terminal: &mut Terminal<CrosstermBackend<std::io::Stdout>>,
) -> anyhow::Result<bool> {
    // Ctrl+D — always quit
    if key.code == crossterm::event::KeyCode::Char('d')
        && key.modifiers.contains(crossterm::event::KeyModifiers::CONTROL)
    {
        return Ok(true);
    }

    Ok(false)
}
