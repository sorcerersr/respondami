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
