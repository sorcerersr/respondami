//! Tests for `streaming_ui::handle_ui_shortcuts`.

use super::streaming_ui::handle_ui_shortcuts;
use crate::tui::{App, AppState, ThinkingDisplay};
use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyEventState, KeyModifiers};

fn make_key(code: KeyCode, modifiers: KeyModifiers) -> KeyEvent {
    KeyEvent {
        code,
        modifiers,
        kind: KeyEventKind::Press,
        state: KeyEventState::NONE,
    }
}

fn make_app() -> App {
    App::new(
        crate::config::Config::default(),
        std::path::PathBuf::from("."),
    )
}

#[test]
fn f1_opens_help_popup() {
    let mut app = make_app();
    assert_eq!(app.modal.state, AppState::Idle);

    let key = make_key(KeyCode::F(1), KeyModifiers::NONE);
    let handled = handle_ui_shortcuts(&mut app, &key).unwrap();

    assert!(handled);
    assert_eq!(app.modal.state, AppState::HelpPopup);
}

#[test]
fn ctrl_o_toggles_thinking_display() {
    let mut app = make_app();
    app.config.thinking_display = ThinkingDisplay::Collapsed;

    let key = make_key(KeyCode::Char('o'), KeyModifiers::CONTROL);
    let handled = handle_ui_shortcuts(&mut app, &key).unwrap();

    assert!(handled);
    assert_eq!(app.config.thinking_display, ThinkingDisplay::Expanded);
}

#[test]
fn ctrl_slash_toggles_thinking_display() {
    let mut app = make_app();
    app.config.thinking_display = ThinkingDisplay::Expanded;

    let key = make_key(KeyCode::Char('/'), KeyModifiers::CONTROL);
    let handled = handle_ui_shortcuts(&mut app, &key).unwrap();

    assert!(handled);
    assert_eq!(app.config.thinking_display, ThinkingDisplay::Hidden);
}

#[test]
fn ctrl_t_toggles_tool_output() {
    let mut app = make_app();

    let key = make_key(KeyCode::Char('t'), KeyModifiers::CONTROL);
    let handled = handle_ui_shortcuts(&mut app, &key).unwrap();

    assert!(handled);
}

#[test]
fn non_matching_key_returns_false() {
    let mut app = make_app();

    let key = make_key(KeyCode::Char('x'), KeyModifiers::NONE);
    let handled = handle_ui_shortcuts(&mut app, &key).unwrap();

    assert!(!handled);
}

#[test]
fn char_o_without_control_returns_false() {
    let mut app = make_app();
    let initial = app.config.thinking_display;

    let key = make_key(KeyCode::Char('o'), KeyModifiers::NONE);
    let handled = handle_ui_shortcuts(&mut app, &key).unwrap();

    assert!(!handled);
    assert_eq!(app.config.thinking_display, initial);
}

#[test]
fn char_t_without_control_returns_false() {
    let mut app = make_app();

    let key = make_key(KeyCode::Char('t'), KeyModifiers::NONE);
    let handled = handle_ui_shortcuts(&mut app, &key).unwrap();

    assert!(!handled);
}
