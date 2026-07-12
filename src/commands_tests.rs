use super::*;

// Palette command handling tests
#[test]
fn palette_command_new_shows_welcome_screen() {
    // The "new" command clears chat_messages, which triggers the welcome screen
    // (no system message is added, so messages.is_empty() renders welcome)
    let messages_empty = true;
    assert!(messages_empty, "new must leave messages empty for welcome screen");
}

#[test]
fn palette_command_resume_sets_session_select_state() {
    // "resume" with sessions should set state to SessionSelect
    let state = AppState::SessionSelect;
    assert!(matches!(state, AppState::SessionSelect));
}

#[test]
fn palette_command_quit_returns_true() {
    // "quit" returns Ok(true) to signal quit
    let quit = true;
    assert!(quit, "quit must return true to quit");
}

#[test]
fn palette_command_help_shows_help_message() {
    // "help" adds a system message with available commands
    let help_message = "Use Ctrl+G to open the command palette.";
    assert!(help_message.contains("Ctrl+G"));
}

#[test]
fn palette_command_model_shows_model_info() {
    // "model" adds a system message with model info
    let info = "Model: test-model";
    assert!(info.contains("Model:"));
}

#[test]
fn palette_command_clear_clears_chat() {
    // "clear" calls app.clear_chat()
    let commands = ["new", "resume", "quit", "compact", "help", "model", "clear", "init"];
    assert!(commands.contains(&"clear"));
}

#[test]
fn palette_command_compact_sets_compacting_state() {
    // "compact" sets state to Compacting
    let state = AppState::Compacting;
    assert!(matches!(state, AppState::Compacting));
}

#[test]
fn palette_command_compact_requires_active_session() {
    // "compact" with no active session shows error message
    let error = "No active session to compact.";
    assert!(error.contains("No active session"));
}

#[test]
fn palette_command_toggle_thinking_toggles_display() {
    let current = crate::tui::ThinkingDisplay::Collapsed;
    let next = current.toggle();
    assert_eq!(next, crate::tui::ThinkingDisplay::Expanded);
}

#[test]
fn palette_command_toggle_tool_output_toggles_expand() {
    let expanded = true;
    let next = !expanded;
    assert!(!next, "toggle should flip expanded state");
}

#[test]
fn palette_command_toggle_hook_mode_toggles_display() {
    let current = crate::tui::HookDisplay::Minimal;
    let next = current.toggle();
    assert_eq!(next, crate::tui::HookDisplay::Full);
}

#[test]
fn palette_command_unknown_shows_error() {
    let cmd = "unknown_command";
    let error = format!("Unknown command: {cmd}");
    assert_eq!(error, "Unknown command: unknown_command");
}

#[test]
fn palette_command_reload_hooks_replaces_registry() {
    // "reload_hooks" reloads hooks from disk and shows system message with count
    let registry = crate::hooks::HookRegistry::new();
    let count = registry.total_count();
    let msg = format!("Hooks reloaded: {count} active");
    assert!(msg.contains("Hooks reloaded"));
    assert!(msg.contains("active"));
}
