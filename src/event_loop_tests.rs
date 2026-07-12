//! Tests for `event_loop` module helpers.

use crate::event_loop::{CompactionResult, transition_to_idle};

// ---- CompactionResult tests ----

#[test]
fn compaction_result_success_display() {
    let result = CompactionResult::Success {
        tokens_before: 1000,
        tokens_after: 500,
        messages_removed: 10,
    };
    assert_eq!(format!("{result}"), "compaction succeeded");
}

#[test]
fn compaction_result_failed_display() {
    let result = CompactionResult::Failed("provider error".to_string());
    assert_eq!(format!("{result}"), "compaction failed: provider error");
}

#[test]
fn compaction_result_panicked_display() {
    let result = CompactionResult::Panicked;
    assert_eq!(format!("{result}"), "compaction task panicked");
}

#[test]
fn compaction_result_debug() {
    let result = CompactionResult::Success {
        tokens_before: 100,
        tokens_after: 50,
        messages_removed: 3,
    };
    let debug = format!("{result:?}");
    assert!(debug.contains("Success"));
    assert!(debug.contains("100"));
}

// ---- transition_to_idle tests ----

#[test]
fn transition_to_idle_sets_idle_state() {
    let mut app = crate::tui::App::new(
        crate::config::Config::default(),
        std::path::PathBuf::from("."),
    );
    app.modal.state = crate::tui::AppState::Compacting;
    transition_to_idle(&mut app);
    assert_eq!(app.modal.state, crate::tui::AppState::Idle);
}

#[test]
fn transition_to_idle_from_streaming() {
    let mut app = crate::tui::App::new(
        crate::config::Config::default(),
        std::path::PathBuf::from("."),
    );
    app.modal.state = crate::tui::AppState::Streaming;
    transition_to_idle(&mut app);
    assert_eq!(app.modal.state, crate::tui::AppState::Idle);
}
