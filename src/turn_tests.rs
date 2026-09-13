//! Tests for turn orchestration — pre-prompt compaction deferral,
//! in-progress guards, and pending-turn rollback.

use std::collections::HashSet;
use std::path::PathBuf;

use ratatui::backend::CrosstermBackend;
use ratatui::Terminal;

use crate::config::Config;
use crate::session::CompactionPlan;
use crate::turn::{rollback_pending_turn, run_turn_with_input, start_turn, PendingTurn};
use crate::tui::{App, AppState, ChatMessage};

/// Build a terminal over stdout. Never drawn to in these tests — the
/// tested paths return before any frame is rendered.
fn make_terminal() -> Terminal<CrosstermBackend<std::io::Stdout>> {
    Terminal::new(CrosstermBackend::new(std::io::stdout())).expect("test terminal")
}

fn make_app() -> App {
    App::new(Config::default(), PathBuf::from("."))
}

/// A compaction task that never finishes — used to trip the in-progress
/// guards. Aborted by the caller at test end.
fn never_finishes_task() -> tokio::task::JoinHandle<anyhow::Result<CompactionPlan>> {
    tokio::spawn(std::future::pending())
}

/// Config that forces the pre-prompt compaction check to fire:
/// tiny context window + zero reserve → `should_compact` is always true.
fn deferral_config() -> Config {
    let mut config = Config::default();
    config.provider.context_window = 10;
    config.compaction.reserve_tokens = 0;
    config
}

// ---- In-progress guards ----

#[tokio::test]
async fn start_turn_blocked_while_compaction_in_progress() {
    let mut app = make_app();
    app.compaction_task = Some(never_finishes_task());
    app.editor.input_buffer = "hello world".to_string();
    let mut terminal = make_terminal();

    let quit = start_turn(&mut app, &mut terminal).await.expect("start_turn");

    assert!(!quit);
    // Typed text stays in the editor — one more Enter re-sends once
    // compaction ends.
    assert_eq!(app.editor.input_buffer, "hello world");
    assert!(app.compaction_task.is_some());
    assert!(app.pending_turn.is_none());
    let (msg, _) = app.ui.status_bar_message.as_ref().expect("status note set");
    assert_eq!(msg, "Compaction in progress — press Esc to cancel");

    if let Some(task) = app.compaction_task.take() {
        task.abort();
    }
}

#[tokio::test]
async fn run_turn_with_input_blocked_while_compaction_in_progress() {
    let mut app = make_app();
    app.compaction_task = Some(never_finishes_task());
    let pre_len = app.chat.chat_messages.len();
    let mut terminal = make_terminal();

    let quit = run_turn_with_input(&mut app, "hello".to_string(), None, HashSet::new(), &mut terminal)
        .await
        .expect("run_turn_with_input");

    assert!(!quit);
    // Nothing added to chat and no turn captured.
    assert_eq!(app.chat.chat_messages.len(), pre_len);
    assert!(app.pending_turn.is_none());
    assert!(app.compaction_task.is_some());
    assert!(app.ui.status_bar_message.is_some());

    if let Some(task) = app.compaction_task.take() {
        task.abort();
    }
}

// ---- Pre-prompt deferral ----

#[tokio::test]
async fn run_turn_defers_to_background_compaction_when_near_threshold() {
    let mut app = App::new(deferral_config(), PathBuf::from("."));
    let pre_len = app.chat.chat_messages.len();
    let mut terminal = make_terminal();

    let quit = run_turn_with_input(&mut app, "hello".to_string(), None, HashSet::new(), &mut terminal)
        .await
        .expect("run_turn_with_input");

    assert!(!quit);
    assert_eq!(app.modal.state, AppState::Compacting);
    assert!(app.compaction_task.is_some());

    let pending = app.pending_turn.take().expect("pending turn captured");
    assert_eq!(pending.input, "hello");
    assert_eq!(pending.restore_input, "hello");
    assert!(pending.created_session);
    assert_eq!(pending.pre_turn_chat_len, pre_len);
    assert!(pending.pre_turn_skills.is_empty());

    // The user message and the compaction note are already visible in chat.
    assert_eq!(app.chat.chat_messages.len(), pre_len + 2);
    assert!(matches!(
        app.chat.chat_messages.last(),
        Some(ChatMessage::System(msg))
            if msg.content == "Context approaching limit. Compacting before sending..."
    ));
    assert!(app.session.session_store.has_active_session());

    // The spawned task sees a header-only session ("Nothing to compact"),
    // so it cannot reach the network. Abort anyway to stay hermetic.
    if let Some(task) = app.compaction_task.take() {
        task.abort();
    }
}

#[tokio::test]
async fn deferral_keeps_typed_text_for_restore_but_launches_with_llm_input() {
    let mut app = App::new(deferral_config(), PathBuf::from("."));
    let mut terminal = make_terminal();

    // `/skillname` shape: LLM gets the injected prompt, chat shows what was
    // typed, and cancel must restore the typed text.
    let input = "Skill: refine\n\n<skill body>".to_string();
    let display = "/refine do the thing".to_string();

    let quit = run_turn_with_input(
        &mut app,
        input.clone(),
        Some(display.clone()),
        HashSet::new(),
        &mut terminal,
    )
    .await
    .expect("run_turn_with_input");

    assert!(!quit);
    let pending = app.pending_turn.take().expect("pending turn captured");
    assert_eq!(pending.input, input);
    assert_eq!(pending.restore_input, display);

    // Chat shows the typed text, not the injected prompt.
    assert!(app
        .chat
        .chat_messages
        .iter()
        .any(|m| matches!(m, ChatMessage::User(u) if u.content == display)));

    if let Some(task) = app.compaction_task.take() {
        task.abort();
    }
}

// ---- Rollback ----

#[test]
fn rollback_pending_turn_restores_pre_turn_state() {
    let mut app = make_app();
    let pre_len = app.chat.chat_messages.len();

    // Simulate the state a deferred turn leaves behind: user message +
    // system note in chat, session auto-created, a skill activated, and
    // the editor holding something the user typed later.
    app.add_user_message("typed question");
    app.add_system_message("Context approaching limit. Compacting before sending...");
    app.session
        .session_store
        .create_session("model".to_string(), 32768, ".".to_string());
    app.active_skills = HashSet::from(["refine".to_string()]);
    app.editor.input_buffer = "later typing".to_string();
    app.editor.cursor_pos = 0;

    app.pending_turn = Some(PendingTurn {
        input: "typed question".to_string(),
        restore_input: "typed question".to_string(),
        created_session: true,
        pre_turn_chat_len: pre_len,
        pre_turn_skills: HashSet::new(),
    });

    rollback_pending_turn(&mut app);

    assert!(app.pending_turn.is_none());
    assert_eq!(app.chat.chat_messages.len(), pre_len);
    // Auto-created session discarded.
    assert!(!app.session.session_store.has_active_session());
    assert!(app.active_skills.is_empty());
    assert_eq!(app.editor.input_buffer, "typed question");
    assert_eq!(app.editor.cursor_pos, "typed question".len());
}

#[test]
fn rollback_pending_turn_keeps_existing_session_when_not_created_by_turn() {
    let mut app = make_app();
    let pre_len = app.chat.chat_messages.len();

    // Session existed before the turn (created_session = false) — rollback
    // must NOT discard it.
    app.session
        .session_store
        .create_session("model".to_string(), 32768, ".".to_string());
    let session_id = app
        .session
        .session_store
        .session_id()
        .expect("active session")
        .to_string();
    app.add_user_message("typed question");

    app.pending_turn = Some(PendingTurn {
        input: "typed question".to_string(),
        restore_input: "typed question".to_string(),
        created_session: false,
        pre_turn_chat_len: pre_len,
        pre_turn_skills: HashSet::new(),
    });

    rollback_pending_turn(&mut app);

    assert!(app.session.session_store.has_active_session());
    assert_eq!(app.session.session_store.session_id(), Some(session_id.as_str()));
    assert_eq!(app.chat.chat_messages.len(), pre_len);
    assert_eq!(app.editor.input_buffer, "typed question");
}

#[test]
fn rollback_pending_turn_is_noop_when_nothing_pending() {
    let mut app = make_app();
    let pre_len = app.chat.chat_messages.len();
    app.editor.input_buffer = "keep me".to_string();

    rollback_pending_turn(&mut app);

    assert!(app.pending_turn.is_none());
    assert_eq!(app.chat.chat_messages.len(), pre_len);
    assert_eq!(app.editor.input_buffer, "keep me");
}

// ---- PendingTurn ----

#[test]
fn pending_turn_derives_debug() {
    let pending = PendingTurn {
        input: "input".to_string(),
        restore_input: "restore".to_string(),
        created_session: true,
        pre_turn_chat_len: 3,
        pre_turn_skills: HashSet::from(["skill".to_string()]),
    };
    let debug = format!("{pending:?}");
    for field in ["input", "restore_input", "created_session", "pre_turn_chat_len", "pre_turn_skills"] {
        assert!(debug.contains(field), "Debug output missing `{field}`: {debug}");
    }
}
