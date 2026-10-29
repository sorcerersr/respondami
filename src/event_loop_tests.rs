//! Tests for `event_loop` module helpers.

use std::collections::HashSet;

use ratatui::backend::CrosstermBackend;
use ratatui::layout::Rect;
use ratatui::{Terminal, TerminalOptions, Viewport};

use crate::event_loop::{CompactionPollResult, CompactionResult, handle_compaction_result, join_panic_message, poll_compaction_task, transition_to_idle};
use crate::session::{CompactionPlan, RequestTokenUsage};
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

// ---- join_panic_message tests ----

#[tokio::test]
async fn join_panic_message_string_payload() {
    let handle: tokio::task::JoinHandle<()> = tokio::spawn(async move {
        panic!("a string panic");
    });
    let err = handle.await.unwrap_err();
    assert!(err.is_panic());
    assert_eq!(join_panic_message(err), "a string panic");
}

#[tokio::test]
async fn join_panic_message_static_str_payload() {
    let handle: tokio::task::JoinHandle<()> = tokio::spawn(async move {
        panic!("a static str panic");
    });
    let err = handle.await.unwrap_err();
    assert!(err.is_panic());
    assert_eq!(join_panic_message(err), "a static str panic");
}

#[tokio::test]
async fn join_panic_message_non_string_falls_back_to_generic() {
    // Panic with a non-string payload (a u32). Both downcasts fail, so the
    // message falls back to the generic "unknown panic payload" string.
    let handle: tokio::task::JoinHandle<()> = tokio::spawn(async move {
        std::panic::panic_any(42u32);
    });
    let err = handle.await.unwrap_err();
    assert!(err.is_panic());
    assert_eq!(join_panic_message(err), "unknown panic payload");
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

/// Build a test terminal that never queries the real terminal size.
///
/// `Terminal::new` uses a fullscreen viewport and calls
/// `crossterm::terminal::size()` — which fails in headless environments
/// (CI runners): there is no controlling tty, stdout is a pipe, and the
/// `tput` fallback is unavailable. `Viewport::Fixed` pins the viewport at
/// construction and `autoresize` is a no-op for fixed viewports, so no OS
/// size query happens at all and the test stays hermetic. Any frames drawn
/// render into the fixed 80×24 area and write to the test's captured
/// stdout. The dimensions are arbitrary — the tested logic never
/// inspects them.
fn make_poll_terminal() -> Terminal<CrosstermBackend<std::io::Stdout>> {
    Terminal::with_options(
        CrosstermBackend::new(std::io::stdout()),
        TerminalOptions {
            viewport: Viewport::Fixed(Rect::new(0, 0, 80, 24)),
        },
    )
    .expect("test terminal")
}

/// A finished plan whose cut index is past the end — `apply_compaction`
/// treats it as a compact-all (empty kept tail), so the rewrite keeps only the
/// summary. Used to exercise the success path without a live LLM.
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
    // An aborted (non-panicking) task is reported as cancelled.
    assert!(matches!(
        app.chat.chat_messages.last(),
        Some(ChatMessage::System(msg)) if msg.content == "Compaction task cancelled."
    ));
}

#[tokio::test]
async fn poll_panicking_task_surfaces_panic_message() {
    let mut app = make_poll_app();
    // A task that panics with a String payload — the JoinError carries the
    // message, which handle_compaction_result must surface in the TUI.
    app.compaction_task = Some(tokio::spawn(async move {
        panic!("byte index 4096 is not a char boundary");
    }));
    yield_to_runtime().await;
    let mut terminal = make_poll_terminal();

    let result = poll_compaction_task(&mut app, &mut terminal).await;

    assert!(matches!(result, CompactionPollResult::Idle));
    assert_eq!(app.modal.state, AppState::Idle);
    assert!(matches!(
        app.chat.chat_messages.last(),
        Some(ChatMessage::System(msg)) if msg.content == "Compaction task panicked: byte index 4096 is not a char boundary"
    ));
}

#[tokio::test]
async fn poll_panicking_task_shows_first_line_only() {
    let mut app = make_poll_app();
    // A multi-line panic payload: the TUI shows only the first line while the
    // full detail goes to the log.
    app.compaction_task = Some(tokio::spawn(async move {
        panic!("first line\nsecond line\nthird line");
    }));
    yield_to_runtime().await;
    let mut terminal = make_poll_terminal();

    let result = poll_compaction_task(&mut app, &mut terminal).await;

    assert!(matches!(result, CompactionPollResult::Idle));
    assert!(matches!(
        app.chat.chat_messages.last(),
        Some(ChatMessage::System(msg)) if msg.content == "Compaction task panicked: first line"
    ));
}

// ---- handle_compaction_result usage-reset tests ----

/// Seed a pre-compaction usage the status bar would otherwise keep showing
/// (`accumulate_token_usage` never decreases, so a stale value sticks).
fn seed_pre_compaction_usage(app: &mut App) {
    app.session.current_request_usage = RequestTokenUsage {
        input_tokens: 5000,
        output_tokens: 300,
        estimated: false,
    };
}

#[tokio::test]
async fn handle_compaction_result_success_resets_request_usage() {
    let mut app = make_poll_app();
    seed_pre_compaction_usage(&mut app);
    let handle = tokio::spawn(async move {
        Ok::<_, anyhow::Error>(noop_plan())
    });

    let result = handle_compaction_result(&mut app, handle, false).await;

    let CompactionResult::Success { tokens_after, .. } = result else {
        panic!("expected success");
    };
    // On success the status-bar usage is reset to the post-compaction estimate.
    assert_eq!(app.session.current_request_usage.input_tokens, tokens_after);
    assert_eq!(app.session.current_request_usage.output_tokens, 0);
    assert!(!app.session.current_request_usage.estimated);
}

#[tokio::test]
async fn handle_compaction_result_failure_keeps_request_usage() {
    let mut app = make_poll_app();
    seed_pre_compaction_usage(&mut app);
    let handle = tokio::spawn(async move {
        Err::<CompactionPlan, anyhow::Error>(anyhow::anyhow!("provider down"))
    });

    let result = handle_compaction_result(&mut app, handle, false).await;

    assert!(matches!(result, CompactionResult::Failed(_)));
    assert_eq!(app.session.current_request_usage.input_tokens, 5000);
    assert_eq!(app.session.current_request_usage.output_tokens, 300);
}

#[tokio::test]
async fn handle_compaction_result_panic_keeps_request_usage() {
    let mut app = make_poll_app();
    seed_pre_compaction_usage(&mut app);
    let handle = tokio::spawn(async move {
        panic!("compaction panic");
    });

    let result = handle_compaction_result(&mut app, handle, false).await;

    assert!(matches!(result, CompactionResult::Panicked));
    assert_eq!(app.session.current_request_usage.input_tokens, 5000);
    assert_eq!(app.session.current_request_usage.output_tokens, 300);
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

// ---- handle_compaction_result PreCompact context tests ----

#[tokio::test]
async fn handle_compaction_result_appends_pending_precompact_context() {
    let mut app = make_poll_app();
    app.session
        .session_store
        .create_session("model".to_string(), 32768, ".".to_string())
        .unwrap();
    app.pending_precompact_context = Some("\n\n[hook: precompact]\nkeep this".to_string());
    let handle = tokio::spawn(async move {
        Ok::<_, anyhow::Error>(noop_plan())
    });

    let result = handle_compaction_result(&mut app, handle, false).await;

    assert!(matches!(result, CompactionResult::Success { .. }));
    // The context is cleared after a single application.
    assert!(app.pending_precompact_context.is_none());
    // The persisted summary carries the appended hook context.
    let summary = app
        .session
        .session_store
        .entries()
        .iter()
        .find_map(|e| match e {
            crate::session::SessionEntry::Compaction { summary, .. } => Some(summary.clone()),
            _ => None,
        })
        .expect("compaction entry persisted");
    assert!(
        summary.starts_with("summary"),
        "summary should start with the base: {summary}"
    );
    assert!(summary.contains("[hook: precompact]"), "summary: {summary}");
    assert!(summary.contains("keep this"), "summary: {summary}");
}

#[tokio::test]
async fn handle_compaction_result_no_pending_context_is_noop() {
    let mut app = make_poll_app();
    app.session
        .session_store
        .create_session("model".to_string(), 32768, ".".to_string())
        .unwrap();
    let handle = tokio::spawn(async move {
        Ok::<_, anyhow::Error>(noop_plan())
    });

    let result = handle_compaction_result(&mut app, handle, false).await;

    assert!(matches!(result, CompactionResult::Success { .. }));
    let summary = app
        .session
        .session_store
        .entries()
        .iter()
        .find_map(|e| match e {
            crate::session::SessionEntry::Compaction { summary, .. } => Some(summary.clone()),
            _ => None,
        })
        .expect("compaction entry persisted");
    assert_eq!(summary, "summary");
}

#[tokio::test]
async fn handle_compaction_result_clears_pending_context_on_failure() {
    let mut app = make_poll_app();
    app.pending_precompact_context = Some("\n\n[hook: precompact]\nx".to_string());
    let handle = tokio::spawn(async move {
        Err::<CompactionPlan, anyhow::Error>(anyhow::anyhow!("provider down"))
    });

    let result = handle_compaction_result(&mut app, handle, false).await;

    assert!(matches!(result, CompactionResult::Failed(_)));
    // Cleared even on failure so stale context never leaks to the next run.
    assert!(app.pending_precompact_context.is_none());
}

// ---- handle_compaction_result strip_images_after_view tests ----

/// Seed a session whose kept tail contains an image tool result. The caller
/// owns `cwd` (a `TempDir`) so the session files stay on disk for the rewrite.
fn app_with_image_tail(cwd: &std::path::Path) -> App {
    let mut app = App::new(crate::config::Config::default(), cwd.to_path_buf());
    app.session
        .session_store
        .create_session("model".to_string(), 32768, "/".to_string())
        .unwrap();
    app.session
        .session_store
        .append_message(None, crate::session::AgentMessage::user("look".to_string()))
        .unwrap();
    app.session
        .session_store
        .append_message(
            None,
            crate::session::AgentMessage::tool(
                "call_img".to_string(),
                "read".to_string(),
                serde_json::json!({ "path": "/tmp/pic.png" }),
                "Image: /tmp/pic.png (100x50, image/png)".to_string(),
                vec![crate::image::ImageContent {
                    data: "aW1hZ2U=".to_string(),
                    mime_type: "image/png".to_string(),
                    width: 100,
                    height: 50,
                }],
            ),
        )
        .unwrap();
    app
}

/// A plan that keeps the tail from the tool message onward (index 2), so the
/// image entry survives the compaction rewrite.
fn keep_tail_plan() -> CompactionPlan {
    CompactionPlan {
        summary: "summary".to_string(),
        cut_index: 2,
        tokens_before: 1000,
    }
}

/// `strip_images_after_view` on: the compaction rewrite also strips the image
/// from the kept entries (the summary is text, so the image can't survive).
#[tokio::test]
async fn handle_compaction_result_strips_images_when_flag_on() {
    let dir = tempfile::TempDir::new().unwrap();
    let mut app = app_with_image_tail(dir.path());
    app.config.config.provider.strip_images_after_view = true;
    let handle = tokio::spawn(async move {
        Ok::<_, anyhow::Error>(keep_tail_plan())
    });

    let result = handle_compaction_result(&mut app, handle, false).await;

    assert!(matches!(result, CompactionResult::Success { .. }));
    let context = app.session.session_store.build_context();
    let tool = context
        .iter()
        .find(|m| matches!(m, crate::session::AgentMessage::Tool { .. }))
        .expect("kept tool message must survive compaction");
    match tool {
        crate::session::AgentMessage::Tool { images, result, .. } => {
            assert!(images.is_empty(), "image must be stripped post-compaction");
            assert!(
                result.contains("[image removed: /tmp/pic.png (100x50). Re-read to view again.]"),
                "removal note must be appended: {result}"
            );
        }
        other => panic!("expected Tool message, got {other:?}"),
    }
}

/// `strip_images_after_view` off (default): the compaction rewrite leaves the
/// image untouched (the workaround is opt-in).
#[tokio::test]
async fn handle_compaction_result_keeps_images_when_flag_off() {
    let dir = tempfile::TempDir::new().unwrap();
    let mut app = app_with_image_tail(dir.path());
    // Default config: strip_images_after_view is false.
    assert!(!app.config.config.provider.strip_images_after_view);
    let handle = tokio::spawn(async move {
        Ok::<_, anyhow::Error>(keep_tail_plan())
    });

    let result = handle_compaction_result(&mut app, handle, false).await;

    assert!(matches!(result, CompactionResult::Success { .. }));
    let context = app.session.session_store.build_context();
    let tool = context
        .iter()
        .find(|m| matches!(m, crate::session::AgentMessage::Tool { .. }))
        .expect("kept tool message must survive compaction");
    match tool {
        crate::session::AgentMessage::Tool { images, .. } => {
            assert_eq!(images.len(), 1, "flag off → image must be kept");
        }
        other => panic!("expected Tool message, got {other:?}"),
    }
}
