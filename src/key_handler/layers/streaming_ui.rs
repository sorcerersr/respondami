//! UI shortcuts that are safe to invoke during streaming.
//!
//! Extracted from `ModalLayer` so both the normal key handler chain and the
//! streaming event loop in `process_agent_events()` can share the same logic.
//!
//! These shortcuts are pure UI operations — they don't touch the agent loop
//! or modify any state that would interfere with LLM work:
//! - `F1` — open help popup
//! - `Ctrl+O` / `Ctrl+/` — toggle thinking/reasoning display
//! - `Ctrl+T` — toggle tool output expand/collapse

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::tui::{App, AppState};

/// Handle UI shortcuts that are safe during streaming.
///
/// Returns `true` if the key was handled, `false` otherwise.
///
/// # Errors
///
/// - Config persistence fails if the config file is unwritable.
pub fn handle_ui_shortcuts(app: &mut App, key: &KeyEvent) -> anyhow::Result<bool> {
    // F1 — open help popup
    if key.code == KeyCode::F(1) {
        app.modal.state = AppState::HelpPopup;
        return Ok(true);
    }

    // Ctrl+O or Ctrl+/: toggle reasoning visibility
    let is_reasoning_toggle = (key.code == KeyCode::Char('o')
        || key.code == KeyCode::Char('/'))
        && key.modifiers.contains(KeyModifiers::CONTROL);
    if is_reasoning_toggle {
        app.config.thinking_display = app.config.thinking_display.toggle();
        app.chat.auto_scroll = true;
        app.save_config()?;
        return Ok(true);
    }

    // Ctrl+T: toggle all tool call output expand/collapse
    let is_expand_toggle = key.code == KeyCode::Char('t')
        && key.modifiers.contains(KeyModifiers::CONTROL);
    if is_expand_toggle {
        app.toggle_all_tool_output();
        app.save_config()?;
        return Ok(true);
    }

    Ok(false)
}
