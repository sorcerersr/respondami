//! Tests for `event_loop` module helpers.

use std::collections::HashSet;

use ratatui::backend::CrosstermBackend;
use ratatui::Terminal;

use crate::event_loop::{CompactionPollResult, CompactionResult, poll_compaction_task, transition_to_idle};
use crate::session::CompactionPlan;
use crate::turn::PendingTurn;
use crate::tui::{App, AppState, ChatMessage};

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

// ---- poll_compaction_task tests ----

fn make_poll_app() -> App {
    App::new(crate::config::Config::default(), std::path::PathBuf::from("."))
}

fn make_poll_terminal() -> Terminal<CrosstermBackend<std::io::Stdout>> {
    Terminal::new(CrosstermBackend::new(std::io::stdout())).expect("test terminal")
}

/// A finished plan with an out-of-range cut index — `apply_compaction`
/// treats it as a no-op, so no session rewrite happens.
fn noop_plan() -> CompactionPlan {
    CompactionPlan {
        summary: "summary".to_string(),
        cut_index: usize::MAX,
        tokens_before: 1000,
    }
}

/// Give the runtime a moment so spawned tasks actually run before we poll.
/// `tokio::spawn` only schedules — without a yield, `is_finished()` is
/// still false when the poll runs.
async fn yield_to_runtime() {
    tokio::time::sleep(std::time::Duration::from_millis(1)).await;
}

#[tokio::test]
async fn poll_not_finished_when_no_task() {
    let mut app = make_poll_app();
    let mut terminal = make_poll_terminal();

    let result = poll_compaction_task(&mut app, &mut terminal).await;

    assert!(matches!(result, CompactionPollResult::NotFinished));
    assert!(app.compaction_task.is_none());
    assert_eq!(app.modal.state, AppState::Idle);
}

#[tokio::test]
async fn poll_not_finished_while_task_running() {
    let mut app = make_poll_app();
    app.compaction_task = Some(tokio::spawn(std::future::pending()));
    let mut terminal = make_poll_terminal();

    let result = poll_compaction_task(&mut app, &mut terminal).await;

    assert!(matches!(result, CompactionPollResult::NotFinished));
    assert!(app.compaction_task.is_some());
    assert_eq!(app.modal.state, AppState::Idle);

    if let Some(task) = app.compaction_task.take() {
        task.abort();
    }
}

#[tokio::test]
async fn poll_manual_compaction_success_returns_idle() {
    let mut app = make_poll_app();
    app.compaction_task = Some(tokio::spawn(async move {
        Ok::<_, anyhow::Error>(noop_plan())
    }));
    yield_to_runtime().await;
    let mut terminal = make_poll_terminal();

    let result = poll_compaction_task(&mut app, &mut terminal).await;

    assert!(matches!(result, CompactionPollResult::Idle));
    assert_eq!(app.modal.state, AppState::Idle);
    assert!(app.compaction_task.is_none());
    assert!(app.pending_turn.is_none());
    // A compaction message was added and the view snapped to bottom.
    assert!(matches!(
        app.chat.chat_messages.last(),
        Some(ChatMessage::Compaction(_))
    ));
    assert!(app.chat.auto_scroll);
}

#[tokio::test]
async fn poll_manual_compaction_failure_returns_idle() {
    let mut app = make_poll_app();
    app.compaction_task = Some(tokio::spawn(async move {
        Err(anyhow::anyhow!("provider down"))
    }));
    yield_to_runtime().await;
    let mut terminal = make_poll_terminal();

    let result = poll_compaction_task(&mut app, &mut terminal).await;

    assert!(matches!(result, CompactionPollResult::Idle));
    assert_eq!(app.modal.state, AppState::Idle);
    assert!(matches!(
        app.chat.chat_messages.last(),
        Some(ChatMessage::System(msg)) if msg.content == "Compaction failed: provider down"
    ));
}

#[tokio::test]
async fn poll_panicked_task_returns_idle() {
    let mut app = make_poll_app();
    let handle = tokio::spawn(async move {
        std::future::pending::<anyhow::Result<CompactionPlan>>().await
    });
    handle.abort();
    app.compaction_task = Some(handle);
    yield_to_runtime().await;
    let mut terminal = make_poll_terminal();

    let result = poll_compaction_task(&mut app, &mut terminal).await;

    assert!(matches!(result, CompactionPollResult::Idle));
    assert_eq!(app.modal.state, AppState::Idle);
    assert!(matches!(
        app.chat.chat_messages.last(),
        Some(ChatMessage::System(msg)) if msg.content == "Compaction task panicked."
    ));
}

#[tokio::test]
async fn poll_launches_pending_turn_when_compaction_finishes() {
    let mut config = crate::config::Config::default();
    // Unreachable provider with retries disabled → the launched agent
    // fails fast on connect, so the turn ends quickly without a live LLM.
    let crate::provider::ProviderSettings::LlamaCpp(settings) = &mut config.provider.settings;
    settings.url = "http://127.0.0.1:1".to_string();
    config.retry.enabled = false;
    let mut app = App::new(config, std::path::PathBuf::from("."));
    app.compaction_task = Some(tokio::spawn(async move {
        Ok::<_, anyhow::Error>(noop_plan())
    }));
    app.pending_turn = Some(PendingTurn {
        input: "deferred question".to_string(),
        restore_input: "deferred question".to_string(),
        created_session: false,
        pre_turn_chat_len: 0,
        pre_turn_skills: HashSet::new(),
    });
    yield_to_runtime().await;
    let mut terminal = make_poll_terminal();

    let result = tokio::time::timeout(
        std::time::Duration::from_secs(15),
        poll_compaction_task(&mut app, &mut terminal),
    )
    .await
    .expect("poll_compaction_task timed out — agent did not fail fast");

    assert!(matches!(
        result,
        CompactionPollResult::TurnLaunched { quit: false }
    ));
    // The turn completed (agent errored immediately) and state is idle.
    assert_eq!(app.modal.state, AppState::Idle);
    assert!(app.compaction_task.is_none());
    assert!(app.pending_turn.is_none());
}
