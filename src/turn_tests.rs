//! Tests for turn orchestration — pre-prompt compaction deferral,
//! in-progress guards, and pending-turn rollback.

use std::collections::HashSet;
use std::path::PathBuf;

use ratatui::backend::CrosstermBackend;
use ratatui::layout::Rect;
use ratatui::{Terminal, TerminalOptions, Viewport};

use crate::config::Config;
use crate::hooks::{Hook, HookEvent, HookSource, ToolMatcher};
use crate::session::CompactionPlan;
use crate::skills::{Skill, SkillSource};
use crate::turn::{rollback_pending_turn, run_turn_with_input, start_turn, PendingTurn};
use crate::tui::{App, AppState, ChatMessage};
use tempfile::TempDir;

/// Build a terminal that never queries the real terminal size.
///
/// `Terminal::new` uses a fullscreen viewport and calls
/// `crossterm::terminal::size()` — which fails in headless environments
/// (CI runners): there is no controlling tty, stdout is a pipe, and the
/// `tput` fallback is unavailable. `Viewport::Fixed` pins the viewport at
/// construction and `autoresize` is a no-op for fixed viewports, so no OS
/// size query happens at all. The tested paths return before any frame is
/// rendered, and the fixed 80×24 area is never inspected by assertions.
fn make_terminal() -> Terminal<CrosstermBackend<std::io::Stdout>> {
    Terminal::with_options(
        CrosstermBackend::new(std::io::stdout()),
        TerminalOptions {
            viewport: Viewport::Fixed(Rect::new(0, 0, 80, 24)),
        },
    )
    .expect("test terminal")
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

/// A directory-style hook that runs a shell command for a given event.
fn make_hook(event: HookEvent, name: &str, command: &str) -> Hook {
    Hook {
        event,
        name: name.to_string(),
        command: command.to_string(),
        source: HookSource::Project,
        skill_name: None,
        skill_dir: None,
        matcher: ToolMatcher::All,
    }
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

// ---- UserPromptSubmit / PreCompact hook injection ----

#[tokio::test]
async fn run_turn_injects_ups_hook_stdout_into_model_input() {
    let mut app = App::new(deferral_config(), PathBuf::from("."));
    app.config
        .hook_registry
        .add(make_hook(HookEvent::UserPromptSubmit, "ups.sh", "echo injected-context"));
    let mut terminal = make_terminal();

    let quit = run_turn_with_input(&mut app, "hello".to_string(), None, HashSet::new(), &mut terminal)
        .await
        .expect("run_turn_with_input");

    assert!(!quit);
    let pending = app.pending_turn.take().expect("pending turn captured");
    // The model-bound input carries the hook context; the display does not.
    assert!(pending.input.starts_with("hello"), "input: {}", pending.input);
    assert!(pending.input.contains("[hook: ups.sh]"), "input: {}", pending.input);
    assert!(pending.input.contains("injected-context"), "input: {}", pending.input);
    assert_eq!(pending.restore_input, "hello");

    // The TUI user message stays clean (no hook context).
    assert!(app
        .chat
        .chat_messages
        .iter()
        .any(|m| matches!(m, ChatMessage::User(u) if u.content == "hello")));

    if let Some(task) = app.compaction_task.take() {
        task.abort();
    }
}

#[tokio::test]
async fn run_turn_ups_hook_empty_stdout_not_injected() {
    let mut app = App::new(deferral_config(), PathBuf::from("."));
    app.config
        .hook_registry
        .add(make_hook(HookEvent::UserPromptSubmit, "ups.sh", "true"));
    let mut terminal = make_terminal();

    let quit = run_turn_with_input(&mut app, "hello".to_string(), None, HashSet::new(), &mut terminal)
        .await
        .expect("run_turn_with_input");

    assert!(!quit);
    let pending = app.pending_turn.take().expect("pending turn captured");
    // No stdout → the model input is exactly the user text.
    assert_eq!(pending.input, "hello");

    if let Some(task) = app.compaction_task.take() {
        task.abort();
    }
}

#[tokio::test]
async fn run_turn_precompact_blocked_undoes_turn_setup() {
    let mut app = App::new(deferral_config(), PathBuf::from("."));
    app.config
        .hook_registry
        .add(make_hook(HookEvent::PreCompact, "gate.sh", "echo no; exit 2"));
    let pre_len = app.chat.chat_messages.len();
    let mut terminal = make_terminal();

    let quit = run_turn_with_input(&mut app, "hello".to_string(), None, HashSet::new(), &mut terminal)
        .await
        .expect("run_turn_with_input");

    assert!(!quit);
    // No compaction and no deferred turn — the turn was vetoed.
    assert!(app.compaction_task.is_none());
    assert!(app.pending_turn.is_none());
    // The turn setup was undone: chat truncated back to pre-turn length plus
    // the single block note, session discarded, editor restored to the typed
    // text.
    assert_eq!(app.chat.chat_messages.len(), pre_len + 1);
    assert!(!app.session.session_store.has_active_session());
    assert_eq!(app.editor.input_buffer, "hello");
    assert_eq!(app.editor.cursor_pos, "hello".len());
    // The block note is the only surviving message.
    assert!(matches!(
        app.chat.chat_messages.last(),
        Some(ChatMessage::System(s)) if s.content.contains("Compaction blocked by hook")
    ));
}

#[tokio::test]
async fn run_turn_precompact_stdout_captured_for_summary() {
    let mut app = App::new(deferral_config(), PathBuf::from("."));
    app.config
        .hook_registry
        .add(make_hook(HookEvent::PreCompact, "pc.sh", "echo preserve-me"));
    let mut terminal = make_terminal();

    let quit = run_turn_with_input(&mut app, "hello".to_string(), None, HashSet::new(), &mut terminal)
        .await
        .expect("run_turn_with_input");

    assert!(!quit);
    // Compaction is deferred and the PreCompact stdout is captured for the
    // summary (applied in `handle_compaction_result`).
    assert!(app.compaction_task.is_some());
    assert!(app.pending_turn.is_some());
    let ctx = app.pending_precompact_context.take().expect("context captured");
    assert!(ctx.contains("[hook: pc.sh]"), "ctx: {ctx}");
    assert!(ctx.contains("preserve-me"), "ctx: {ctx}");

    if let Some(task) = app.compaction_task.take() {
        task.abort();
    }
}

// ---- /skillname activation persistence ----

#[tokio::test]
async fn skill_activation_persists_to_session_header() {
    let tmp = TempDir::new().unwrap();
    let skill_dir = tmp.path().join("my-skill");
    std::fs::create_dir_all(&skill_dir).unwrap();
    let skill_path = skill_dir.join("SKILL.md");
    std::fs::write(&skill_path, "---\nname: my-skill\ndescription: test\n---\nSkill body.").unwrap();

    let mut app = App::new(deferral_config(), PathBuf::from("."));
    app.config.skills = vec![Skill {
        name: "my-skill".to_string(),
        description: "test".to_string(),
        license: None,
        metadata: std::collections::HashMap::new(),
        hooks: Vec::new(),
        file_path: skill_path,
        base_dir: skill_dir,
        source: SkillSource::Global,
    }];
    // Existing session: the header must receive the activation.
    app.session
        .session_store
        .create_session("model".to_string(), 32768, ".".to_string())
        .unwrap();

    app.editor.input_buffer = "/my-skill".to_string();
    let mut terminal = make_terminal();

    let quit = start_turn(&mut app, &mut terminal).await.expect("start_turn");

    assert!(!quit);
    assert!(app.active_skills.contains("my-skill"));
    assert!(app
        .session
        .session_store
        .get_activated_skills()
        .contains(&"my-skill".to_string()));

    // The tiny context window deferred the turn to background compaction;
    // the header-only session makes it a no-op. Abort to stay hermetic.
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
        .create_session("model".to_string(), 32768, ".".to_string()).unwrap();
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
        .create_session("model".to_string(), 32768, ".".to_string()).unwrap();
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
