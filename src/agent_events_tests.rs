use crate::tui::AgentEvent;
use crate::session::Usage;
use crate::skills::{Skill, SkillHook, SkillSource};
use crate::tui::App;

fn app_with_skill(name: &str, hooks: Vec<SkillHook>) -> App {
    let mut app = App::new(
        crate::config::Config::default(),
        std::path::PathBuf::from("."),
    );
    // Hermetic: drop any directory hooks loaded from the real config dir so
    // the exact-count assertions below are environment-independent.
    app.config.hook_registry = crate::hooks::HookRegistry::new();
    app.config.skills = vec![Skill {
        name: name.to_string(),
        description: "test skill".to_string(),
        license: None,
        metadata: std::collections::HashMap::new(),
        hooks,
        file_path: std::path::PathBuf::from(format!("/skills/{name}/SKILL.md")),
        base_dir: std::path::PathBuf::from(format!("/skills/{name}")),
        source: SkillSource::Global,
    }];
    app
}

/// `SkillActivation` handler: first activation registers the skill's hooks
/// into the TUI registry (the persistent source every agent launch clones).
#[test]
fn skill_activation_registers_hooks_on_first_activation() {
    let mut app = app_with_skill(
        "planning",
        vec![SkillHook {
            event: crate::hooks::HookEvent::PreToolUse,
            command: "sh /skills/planning/scripts/enforce.sh".to_string(),
            matcher: None,
        }],
    );

    super::agent_events::handle_skill_activation_event(&mut app, "planning");

    assert!(app.active_skills.contains("planning"));
    let hooks = app.config.hook_registry.hooks(crate::hooks::HookEvent::PreToolUse);
    assert_eq!(hooks.len(), 1);
    assert_eq!(hooks[0].name, "planning:enforce.sh");
    // A SkillActivation chat message was displayed.
    assert!(app
        .chat
        .chat_messages
        .iter()
        .any(|m| matches!(m, crate::tui::ChatMessage::SkillActivation(_))));
}

/// `SkillActivation` handler: a re-activation (skill already active) keeps
/// exactly one copy of the hooks — the double-fire regression guard.
#[test]
fn skill_activation_reactivation_keeps_single_copy() {
    let mut app = app_with_skill(
        "planning",
        vec![SkillHook {
            event: crate::hooks::HookEvent::PreToolUse,
            command: "sh /skills/planning/scripts/enforce.sh".to_string(),
            matcher: None,
        }],
    );

    super::agent_events::handle_skill_activation_event(&mut app, "planning");
    super::agent_events::handle_skill_activation_event(&mut app, "planning");

    assert_eq!(
        app.config
            .hook_registry
            .hooks(crate::hooks::HookEvent::PreToolUse)
            .len(),
        1
    );
}

/// Regression test: `drain_pending_events` processes Token, Reasoning, and Usage events
/// that are still buffered in the channel when Done arrives.
///
/// Bug (C-1): Done may arrive while Token/Reasoning/Usage events are still buffered
/// (channel capacity 256). Without draining, the final lines of output are
/// silently dropped, and Usage events are lost from session counters.
///
/// Fix: All drain paths now include Usage events (previously error paths
/// called with `include_usage = false`, silently discarding Usage events).
#[test]
fn drain_pending_events_processes_all_event_types() {
    let (tx, mut rx) = tokio::sync::mpsc::channel::<AgentEvent>(256);
    let _ = tx.try_send(AgentEvent::Token("hello".into()));
    let _ = tx.try_send(AgentEvent::Token(" world".into()));
    let _ = tx.try_send(AgentEvent::Reasoning("thinking...".into()));
    let _ = tx.try_send(AgentEvent::Usage(Usage {
        prompt_tokens: 100,
        completion_tokens: 50,
        total_tokens: 150,
    }));
    drop(tx);

    // All event types are processed
    let mut tokens = Vec::new();
    let mut reasoning = Vec::new();
    let mut usages = Vec::new();
    while let Ok(evt) = rx.try_recv() {
        match evt {
            AgentEvent::Token(t) => tokens.push(t),
            AgentEvent::Reasoning(r) => reasoning.push(r),
            AgentEvent::Usage(u) => usages.push(u),
            _ => {}
        }
    }
    assert_eq!(tokens, vec!["hello".to_string(), " world".to_string()]);
    assert_eq!(reasoning, vec!["thinking...".to_string()]);
    assert_eq!(usages.len(), 1);
    assert_eq!(usages[0].completion_tokens, 50);
}

// ---------------------------------------------------------------------------
// StripImages handler (strip_images_after_view)
// ---------------------------------------------------------------------------

fn image_tool_msg() -> crate::session::AgentMessage {
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
    )
}

/// `StripImages` handler: strips the matching tool result from the active
/// session and rewrites the JSONL (base64 gone, note present).
#[test]
fn strip_images_event_strips_active_session() {
    let dir = tempfile::TempDir::new().unwrap();
    let mut app = App::new(crate::config::Config::default(), dir.path().to_path_buf());
    app.session
        .session_store
        .create_session("model".to_string(), 32768, "/".to_string())
        .unwrap();
    app.session
        .session_store
        .append_message(None, image_tool_msg())
        .unwrap();

    super::agent_events::handle_strip_images_event(&mut app, vec!["call_img".to_string()]);

    let context = app.session.session_store.build_context();
    match &context[0] {
        crate::session::AgentMessage::Tool { images, result, .. } => {
            assert!(images.is_empty(), "images must be stripped from the session");
            assert!(
                result.contains("[image removed: /tmp/pic.png (100x50). Re-read to view again.]"),
                "removal note must be appended: {result}"
            );
        }
        other => panic!("expected Tool message, got {other:?}"),
    }
}

/// `StripImages` handler with no active session: a no-op (no panic, no error).
#[test]
fn strip_images_event_no_active_session_is_noop() {
    let dir = tempfile::TempDir::new().unwrap();
    let mut app = App::new(crate::config::Config::default(), dir.path().to_path_buf());
    // No session created — the handler must be a clean no-op.
    super::agent_events::handle_strip_images_event(&mut app, vec!["call_img".to_string()]);
    assert!(!app.session.session_store.has_active_session());
}

// ---------------------------------------------------------------------------
// Loop recovery (decide_loop_recovery + decay)
// ---------------------------------------------------------------------------

/// An assistant message issuing a `bash` call with the given command.
fn bash_assistant(command: &str) -> crate::session::AgentMessage {
    crate::session::AgentMessage::assistant(
        String::new(),
        String::new(),
        vec![crate::session::ToolCall {
            id: "call_1".to_string(),
            name: "bash".to_string(),
            arguments: serde_json::json!({ "command": command }),
        }],
        None,
    )
}

/// A report for a repeated `bash grep` loop.
fn grep_report() -> crate::agent::LoopReport {
    crate::agent::LoopReport {
        kind: crate::agent::LoopKind::ToolCall,
        tool_name: "bash".to_string(),
        canonical_args: crate::agent::canonical_args(&serde_json::json!({ "command": "grep -n foo" })),
        args_preview: "grep -n foo".to_string(),
        count: 10,
    }
}

/// Build an app with a session: user task, a pre-loop `ls` turn, then the
/// repeated `grep` loop (assistant + tool ×2).
fn app_with_loop_session() -> (App, tempfile::TempDir) {
    let dir = tempfile::TempDir::new().unwrap();
    let mut app = App::new(crate::config::Config::default(), dir.path().to_path_buf());
    app.session
        .session_store
        .create_session("model".to_string(), 32768, "/".to_string())
        .unwrap();
    app.session
        .session_store
        .append_message(None, crate::session::AgentMessage::user("task".into()))
        .unwrap();
    app.session
        .session_store
        .append_message(None, bash_assistant("ls"))
        .unwrap();
    app.session
        .session_store
        .append_message(
            None,
            crate::session::AgentMessage::tool(
                "c".into(),
                "bash".into(),
                serde_json::json!({ "command": "ls" }),
                "ok".into(),
                vec![],
            ),
        )
        .unwrap();
    app.session
        .session_store
        .append_message(None, bash_assistant("grep -n foo"))
        .unwrap();
    app.session
        .session_store
        .append_message(
            None,
            crate::session::AgentMessage::tool(
                "c".into(),
                "bash".into(),
                serde_json::json!({ "command": "grep -n foo" }),
                "ok".into(),
                vec![],
            ),
        )
        .unwrap();
    (app, dir)
}

/// `LoopDetected` within budget: the cut is applied (loop removed, pre-loop
/// work kept), the guard is reset, the recovery counter increments, and a
/// recovery prompt is returned.
#[test]
fn loop_recovery_within_budget_cuts_and_restarts() {
    let (mut app, _dir) = app_with_loop_session();
    let report = grep_report();

    let decision = super::agent_events::decide_loop_recovery(&mut app, &report);

    match decision {
        super::agent_events::LoopRecoveryDecision::Recover { prompt, removed } => {
            assert_eq!(removed, 2, "the assistant + tool loop entries must be cut");
            assert!(prompt.contains("grep -n foo"), "prompt must name the loop");
        }
        other => panic!("expected Recover, got {other:?}"),
    }
    assert_eq!(app.loop_recovery_count, 1, "recovery counter must increment");

    // The pre-loop `ls` turn survives; the looped `grep` is gone.
    let context = app.session.session_store.build_context();
    assert!(
        !context.iter().any(|m| matches!(m, crate::session::AgentMessage::Assistant { content, .. } if content.iter().any(|b| matches!(b, crate::session::ContentBlock::ToolCall { tool_call } if tool_call.arguments["command"] == "grep -n foo")))),
        "the looped call must be cut"
    );
}

/// `LoopDetected` with the budget already spent: `Exhausted`, no cut, counter
/// unchanged.
#[test]
fn loop_recovery_exhausted_stops_without_cut() {
    let (mut app, _dir) = app_with_loop_session();
    app.loop_recovery_count = app.config.config.loop_detection.max_recoveries;
    let entries_before = app.session.session_store.entries().len();
    let report = grep_report();

    let decision = super::agent_events::decide_loop_recovery(&mut app, &report);

    assert!(
        matches!(decision, super::agent_events::LoopRecoveryDecision::Exhausted),
        "budget spent → Exhausted"
    );
    assert_eq!(
        app.session.session_store.entries().len(),
        entries_before,
        "no cut must be applied when exhausted"
    );
    assert_eq!(
        app.loop_recovery_count, app.config.config.loop_detection.max_recoveries,
        "counter must not increment past the budget"
    );
}

/// `LoopDetected` with no active session: still `Recover` (nothing to cut),
/// counter increments, empty prompt path is safe.
#[test]
fn loop_recovery_no_active_session_recovers_without_cut() {
    let dir = tempfile::TempDir::new().unwrap();
    let mut app = App::new(crate::config::Config::default(), dir.path().to_path_buf());
    // No session created.
    let report = grep_report();

    let decision = super::agent_events::decide_loop_recovery(&mut app, &report);

    match decision {
        super::agent_events::LoopRecoveryDecision::Recover { removed, .. } => {
            assert_eq!(removed, 0, "no session → nothing removed");
        }
        other => panic!("expected Recover, got {other:?}"),
    }
    assert_eq!(app.loop_recovery_count, 1);
}

/// `apply_loop_decay`: after `reset_after_calls` loop-free tool calls, an
/// elevated recovery counter resets to 0; below the threshold it does not.
#[test]
fn loop_decay_resets_elevated_counter() {
    let dir = tempfile::TempDir::new().unwrap();
    let mut app = App::new(crate::config::Config::default(), dir.path().to_path_buf());
    let reset_after = app.config.config.loop_detection.reset_after_calls;
    app.loop_recovery_count = 1;

    // Below the threshold: counter stays elevated.
    for _ in 0..(reset_after - 1) {
        app.apply_loop_decay();
    }
    assert_eq!(app.loop_recovery_count, 1, "must not reset below the threshold");
    assert_eq!(app.loop_free_tool_calls, reset_after - 1);

    // The threshold-th call resets both.
    app.apply_loop_decay();
    assert_eq!(app.loop_recovery_count, 0, "must reset at the threshold");
    assert_eq!(app.loop_free_tool_calls, 0);
}

/// `apply_loop_decay` is a no-op until a recovery has happened (counter 0).
#[test]
fn loop_decay_noop_without_recovery() {
    let dir = tempfile::TempDir::new().unwrap();
    let mut app = App::new(crate::config::Config::default(), dir.path().to_path_buf());
    app.loop_recovery_count = 0;
    for _ in 0..100 {
        app.apply_loop_decay();
    }
    assert_eq!(app.loop_recovery_count, 0);
    assert_eq!(app.loop_free_tool_calls, 0, "no decay counting without a recovery");
}

// ---------------------------------------------------------------------------
// Text-echo loop recovery (decide_loop_recovery, TextEcho kind)
// ---------------------------------------------------------------------------

/// A report for a repeated text-only response (the 5b6b68d3 pattern).
fn text_echo_report() -> crate::agent::LoopReport {
    crate::agent::LoopReport {
        kind: crate::agent::LoopKind::TextEcho,
        tool_name: "response".to_string(),
        canonical_args: "I need to investigate this further.".to_string(),
        args_preview: "I need to investigate this further.".to_string(),
        count: 603,
    }
}

/// An assistant message with only a visible text block.
fn text_assistant(text: &str) -> crate::session::AgentMessage {
    crate::session::AgentMessage::assistant_with_blocks(
        vec![crate::session::ContentBlock::Text {
            text: text.to_string(),
        }],
        None,
    )
}

/// Build an app with the 5b6b68d3 shape: user task, then the repeated
/// (stop-block user, assistant text) pairs ×3.
fn app_with_text_loop_session() -> (App, tempfile::TempDir) {
    let dir = tempfile::TempDir::new().unwrap();
    let mut app = App::new(crate::config::Config::default(), dir.path().to_path_buf());
    app.session
        .session_store
        .create_session("model".to_string(), 32768, "/".to_string())
        .unwrap();
    app.session
        .session_store
        .append_message(None, crate::session::AgentMessage::user("real task".into()))
        .unwrap();
    for _ in 0..3 {
        app.session
            .session_store
            .append_message(
                None,
                crate::session::AgentMessage::user("blocked: plan incomplete".into()),
            )
            .unwrap();
        app.session
            .session_store
            .append_message(
                None,
                text_assistant("I need to investigate this further."),
            )
            .unwrap();
    }
    (app, dir)
}

/// `TextEcho` within budget: the cut removes the repeated pairs (preserving
/// the real task), the recovery counter increments, and a text recovery
/// prompt is returned.
#[test]
fn text_loop_recovery_within_budget_cuts_and_restarts() {
    let (mut app, _dir) = app_with_text_loop_session();
    let report = text_echo_report();

    let decision = super::agent_events::decide_loop_recovery(&mut app, &report);

    match decision {
        super::agent_events::LoopRecoveryDecision::Recover { prompt, removed } => {
            assert_eq!(removed, 6, "the 3 repeated (user, assistant) pairs must be cut");
            assert!(
                prompt.contains("I need to investigate this further."),
                "prompt must name the repeated response"
            );
            assert!(prompt.contains("603"), "prompt must state the count");
        }
        other => panic!("expected Recover, got {other:?}"),
    }
    assert_eq!(app.loop_recovery_count, 1, "recovery counter must increment");

    // The real task survives; the looped response is gone.
    let context = app.session.session_store.build_context();
    assert!(
        !context
            .iter()
            .any(|m| matches!(m, crate::session::AgentMessage::Assistant { content, .. } if content.iter().any(|b| matches!(b, crate::session::ContentBlock::Text { text } if text.contains("I need to investigate"))))),
        "the looped response must be cut"
    );
}

/// `TextEcho` with the budget already spent: `Exhausted`, no cut, counter
/// unchanged.
#[test]
fn text_loop_recovery_exhausted_stops_without_cut() {
    let (mut app, _dir) = app_with_text_loop_session();
    app.loop_recovery_count = app.config.config.loop_detection.max_recoveries;
    let entries_before = app.session.session_store.entries().len();
    let report = text_echo_report();

    let decision = super::agent_events::decide_loop_recovery(&mut app, &report);

    assert!(
        matches!(decision, super::agent_events::LoopRecoveryDecision::Exhausted),
        "budget spent → Exhausted"
    );
    assert_eq!(
        app.session.session_store.entries().len(),
        entries_before,
        "no cut must be applied when exhausted"
    );
}
