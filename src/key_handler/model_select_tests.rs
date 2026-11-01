// Tests for the model select cancel behavior.
//
// Cancelling the model select must change nothing: no system message is
// emitted (one would clobber the welcome screen and add no information to an
// existing conversation) and the app returns to Idle.

use crate::config::Config;
use crate::key_handler::model_select::do_cancel_model_select;
use crate::tui::{App, AppState};

fn make_app() -> App {
    App::new(Config::default(), std::path::PathBuf::from("."))
}

#[test]
fn cancel_returns_to_idle() {
    let mut app = make_app();
    app.modal.state = AppState::ModelSelect;
    do_cancel_model_select(&mut app);
    assert_eq!(app.modal.state, AppState::Idle);
}

#[test]
fn cancel_adds_no_message_to_empty_chat() {
    let mut app = make_app();
    app.modal.state = AppState::ModelSelect;
    let before = app.chat.chat_messages.len();
    do_cancel_model_select(&mut app);
    assert_eq!(
        app.chat.chat_messages.len(),
        before,
        "cancelling must not add a message (welcome screen would be clobbered)"
    );
}

#[test]
fn cancel_adds_no_message_to_nonempty_chat() {
    let mut app = make_app();
    app.add_user_message("hello");
    app.modal.state = AppState::ModelSelect;
    let before = app.chat.chat_messages.len();
    do_cancel_model_select(&mut app);
    assert_eq!(
        app.chat.chat_messages.len(),
        before,
        "cancelling must not add a message to an existing conversation"
    );
    assert_eq!(app.modal.state, AppState::Idle);
}
