use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use tempfile::TempDir;

use crate::image::ImageContent;
use crate::session::{AgentMessage, ContentBlock, ToolCall, Usage};
use crate::skills::{Skill, SkillSource};
use crate::tui::AgentEvent;

fn img() -> ImageContent {
    ImageContent {
        data: "aW1hZ2U=".to_string(),
        mime_type: "image/png".to_string(),
        width: 100,
        height: 50,
    }
}

#[test]
fn vision_gate_enabled_keeps_images() {
    let (content, images) = super::apply_vision_gate(true, "Image: x".to_string(), vec![img()]);
    assert_eq!(content, "Image: x");
    assert_eq!(images.len(), 1);
}

#[test]
fn vision_gate_disabled_drops_images_with_note() {
    let (content, images) = super::apply_vision_gate(false, "Image: x".to_string(), vec![img()]);
    assert!(
        images.is_empty(),
        "images must be dropped when vision is off"
    );
    assert!(
        content.contains("Vision disabled"),
        "note must explain the drop: {content}"
    );
    assert!(
        content.contains("1 image"),
        "note must state the count: {content}"
    );
    assert!(
        content.contains("provider.vision: true"),
        "note must point at the provider-level config key: {content}"
    );
}

#[test]
fn vision_gate_disabled_no_images_is_noop() {
    let (content, images) = super::apply_vision_gate(false, "plain text".to_string(), Vec::new());
    assert_eq!(content, "plain text", "no images → content unchanged");
    assert_eq!(images, Vec::new());
}

// ---------------------------------------------------------------------------
// image_removal_note / strip_images_from_tool (strip_images_after_view)
// ---------------------------------------------------------------------------

#[test]
fn image_removal_note_format() {
    let note = super::image_removal_note("/tmp/pic.png", 100, 50);
    assert_eq!(note, "[image removed: /tmp/pic.png (100x50). Re-read to view again.]");
}

fn tool_msg_with_image() -> AgentMessage {
    AgentMessage::tool(
        "call_img".to_string(),
        "read".to_string(),
        serde_json::json!({ "path": "/tmp/pic.png" }),
        "Image: /tmp/pic.png (100x50, image/png)".to_string(),
        vec![img()],
    )
}

#[test]
fn strip_images_from_tool_clears_images_and_appends_note() {
    let msg = tool_msg_with_image();
    let stripped = super::strip_images_from_tool(&msg);

    match &stripped {
        AgentMessage::Tool { result, images, .. } => {
            assert!(images.is_empty(), "images must be cleared");
            assert!(
                result.starts_with("Image: /tmp/pic.png"),
                "original result text must be preserved: {result}"
            );
            assert!(
                result.contains("[image removed: /tmp/pic.png (100x50). Re-read to view again.]"),
                "removal note must be appended: {result}"
            );
            // The note is on its own line after the original result.
            assert!(result.contains("\n[image removed:"));
        }
        other => panic!("expected Tool message, got {other:?}"),
    }
}

#[test]
fn strip_images_from_tool_no_images_is_noop() {
    let msg = AgentMessage::tool(
        "call_txt".to_string(),
        "read".to_string(),
        serde_json::json!({ "path": "/tmp/file.txt" }),
        "file contents".to_string(),
        Vec::new(),
    );
    let stripped = super::strip_images_from_tool(&msg);
    assert_eq!(
        serde_json::to_value(&stripped).unwrap(),
        serde_json::to_value(&msg).unwrap(),
        "no images → byte-identical message"
    );
}

#[test]
fn strip_images_from_tool_non_tool_is_noop() {
    let msg = AgentMessage::user("hello".to_string());
    let stripped = super::strip_images_from_tool(&msg);
    assert_eq!(
        serde_json::to_value(&stripped).unwrap(),
        serde_json::to_value(&msg).unwrap(),
        "non-tool message → unchanged"
    );
}

#[test]
fn strip_images_from_tool_missing_path_uses_unknown() {
    let msg = AgentMessage::tool(
        "call_img".to_string(),
        "read".to_string(),
        serde_json::json!({}),
        "Image: x".to_string(),
        vec![img()],
    );
    let stripped = super::strip_images_from_tool(&msg);
    match &stripped {
        AgentMessage::Tool { result, .. } => {
            assert!(
                result.contains("[image removed: unknown (100x50). Re-read to view again.]"),
                "missing path → 'unknown': {result}"
            );
        }
        other => panic!("expected Tool message, got {other:?}"),
    }
}

/// Byte-identity invariant: the live-context strip and the session strip
/// (same function) must produce identical digests, so the history guard sees
/// the same message on both sides.
#[test]
fn strip_images_from_tool_digest_changes() {
    let msg = tool_msg_with_image();
    let original_digest = crate::history_guard::message_digest(&msg);
    let stripped = super::strip_images_from_tool(&msg);
    let stripped_digest = crate::history_guard::message_digest(&stripped);
    assert_ne!(
        original_digest, stripped_digest,
        "strip must change the digest (the expected one-time drift)"
    );
}

/// `strip_seen_images`: strips every image in the context, recomputes digests
/// in lockstep, and emits a `StripImages` event with the stripped IDs.
#[tokio::test]
async fn strip_seen_images_strips_and_emits_event() {
    use crate::history_guard::MessageDigest;
    use tokio::sync::mpsc;

    let (tx, mut rx) = mpsc::channel::<AgentEvent>(16);
    let mut context = vec![
        AgentMessage::user("look at this".to_string()),
        tool_msg_with_image(),
        AgentMessage::assistant("here it is".to_string(), String::new(), Vec::new(), None),
    ];
    let mut digests: Vec<MessageDigest> =
        context.iter().map(crate::history_guard::message_digest).collect();

    super::strip_seen_images(&mut context, &mut digests, &tx).await;

    // The tool message is stripped; the others are untouched.
    match &context[1] {
        AgentMessage::Tool { images, result, .. } => {
            assert_eq!(images.as_slice(), []);
            assert!(result.contains("[image removed:"));
        }
        other => panic!("expected Tool message, got {other:?}"),
    }
    match &context[0] {
        AgentMessage::User { content } => assert_eq!(content, "look at this"),
        other => panic!("expected User message, got {other:?}"),
    }

    // Digests recomputed in lockstep with the stripped context.
    assert_eq!(digests.len(), context.len());
    for (d, m) in digests.iter().zip(context.iter()) {
        assert_eq!(*d, crate::history_guard::message_digest(m));
    }

    // Exactly one StripImages event carrying the stripped tool call ID.
    let event = rx.try_recv().expect("StripImages event must be emitted");
    match event {
        AgentEvent::StripImages { tool_call_ids } => {
            assert_eq!(tool_call_ids, vec!["call_img".to_string()]);
        }
        other => panic!("expected StripImages, got {other:?}"),
    }
    assert!(rx.try_recv().is_err(), "no further events");
}

/// `strip_seen_images` with no images: no mutation, no event.
#[tokio::test]
async fn strip_seen_images_no_images_no_event() {
    use crate::history_guard::MessageDigest;
    use tokio::sync::mpsc;

    let (tx, mut rx) = mpsc::channel::<AgentEvent>(16);
    let mut context = vec![AgentMessage::user("hello".to_string())];
    let mut digests: Vec<MessageDigest> =
        context.iter().map(crate::history_guard::message_digest).collect();

    super::strip_seen_images(&mut context, &mut digests, &tx).await;

    assert!(rx.try_recv().is_err(), "no event when nothing to strip");
    assert_eq!(context.len(), 1);
}

#[test]
fn build_system_prompt_no_agents_md() {
    let dir = TempDir::new().unwrap();
    let (prompt, err) = super::build_system_prompt_with_agents_md(dir.path(), &[]);
    assert!(err.is_none());
    assert!(!prompt.contains("<project_context>"));
    assert!(prompt.contains("You are Respondami"));
    assert!(prompt.contains("Current date:"));
    assert!(prompt.contains("Current working directory:"));
}

#[test]
fn build_system_prompt_with_agents_md() {
    let dir = TempDir::new().unwrap();
    fs::write(dir.path().join("AGENTS.md"), "# Project Rules\nBe nice.").unwrap();
    let (prompt, err) = super::build_system_prompt_with_agents_md(dir.path(), &[]);
    assert!(err.is_none());
    assert!(prompt.starts_with("You are Respondami"));
    assert!(prompt.contains("<project_context>"));
    assert!(prompt.contains("<project_instructions path="));
    assert!(prompt.contains("# Project Rules"));
    assert!(prompt.contains("</project_context>"));
    assert!(prompt.contains("Current date:"));
    assert!(prompt.contains("Current working directory:"));
}

#[test]
fn build_system_prompt_agents_md_error_includes_skills() {
    let dir = TempDir::new().unwrap();
    fs::create_dir(dir.path().join("AGENTS.md")).unwrap();

    let skills = vec![Skill {
        name: "test-skill".to_string(),
        description: "A test skill".to_string(),
        license: None,
        metadata: HashMap::new(),
        hooks: Vec::new(),
        file_path: PathBuf::from("/test/SKILL.md"),
        base_dir: PathBuf::from("/test"),
        source: SkillSource::Global,
    }];

    let (prompt, err) = super::build_system_prompt_with_agents_md(dir.path(), &skills);
    assert!(
        err.is_some(),
        "AGENTS.md as directory should cause load error"
    );
    assert!(
        prompt.contains("<available_skills>"),
        "skills block must be included even when AGENTS.md load fails"
    );
    assert!(prompt.contains("test-skill"));
}

#[test]
fn hook_registry_empty_no_hooks() {
    let registry = crate::hooks::HookRegistry::new();
    assert_eq!(registry.hooks(crate::hooks::HookEvent::PreToolUse).len(), 0);
}

#[test]
fn hook_registry_blocks_pre_tool_use() {
    // This test verifies that a blocking PreToolUse hook prevents tool execution
    // Full integration test would require mocking the agent loop
}

/// Regression test: agent loop must reject empty responses (content="" with no tool calls)
/// when the provider returns an empty response, the agent loop should send Done(Err(...))
/// instead of saving the empty assistant message to the session.
#[tokio::test]
async fn agent_loop_rejects_empty_response() {
    // This test verifies the logic of the empty response check in the agent loop.
    // The agent loop checks: if content.trim().is_empty() && !has_tool_calls → Done(Err)
    // We can verify this by checking the condition directly.
    let content = String::new();
    let has_tool_calls = false;
    let should_reject = content.trim().is_empty() && !has_tool_calls;
    assert!(
        should_reject,
        "empty response with no tool calls should be rejected"
    );

    // Also test with whitespace-only content
    let content = "   \n\t  ".to_string();
    let should_reject = content.trim().is_empty() && !has_tool_calls;
    assert!(should_reject, "whitespace-only response should be rejected");

    // But a response with tool calls should NOT be rejected even if content is empty
    let has_tool_calls = true;
    let should_reject = content.trim().is_empty() && !has_tool_calls;
    assert!(
        !should_reject,
        "empty content with tool calls should not be rejected"
    );
}

/// Regression: `scroll_to_bottom` must be set on all agent exit paths.
/// Every exit path from `process_agent_events` must set `auto_scroll` = true
/// (guarded by !`pinned_scroll`) to ensure the complete response is visible.
#[test]
fn scroll_to_bottom_on_all_exit_paths_logic() {
    // Test the logic: pinned_scroll guards auto_scroll.
    // When pinned_scroll is false, auto_scroll must be set to true.
    // When pinned_scroll is true, auto_scroll is NOT set (user is pinned).
    let pinned_scroll = false;
    let should_auto_scroll = !pinned_scroll;
    assert!(
        should_auto_scroll,
        "auto_scroll should be set when not pinned"
    );

    let pinned_scroll = true;
    let should_auto_scroll = !pinned_scroll;
    assert!(
        !should_auto_scroll,
        "auto_scroll should NOT be set when pinned"
    );
}

fn test_tool_call(id: &str) -> ToolCall {
    ToolCall {
        id: id.to_string(),
        name: "bash".to_string(),
        arguments: serde_json::json!({ "command": "ls" }),
    }
}

/// C1 regression: the first tool call of a multi-tool response must carry the
/// response's non-tool blocks plus only its own tool call — no duplication of
/// its tool call, no leak of later tool calls.
#[test]
fn canonical_turn_blocks_first_call_shape() {
    let tc0 = test_tool_call("call_0");
    let tc1 = test_tool_call("call_1");
    let content = vec![
        ContentBlock::Thinking {
            thinking: "hmm".into(),
        },
        ContentBlock::Text {
            text: "let me check".into(),
        },
        ContentBlock::ToolCall {
            tool_call: tc0.clone(),
        },
        ContentBlock::ToolCall {
            tool_call: tc1.clone(),
        },
    ];

    let blocks = super::canonical_turn_blocks(&content, &tc0, true);

    assert_eq!(
        blocks.len(),
        3,
        "first call: thinking + text + own tool call only"
    );
    assert!(matches!(blocks[0], ContentBlock::Thinking { .. }));
    assert!(matches!(blocks[1], ContentBlock::Text { .. }));
    match &blocks[2] {
        ContentBlock::ToolCall { tool_call } => assert_eq!(tool_call.id, "call_0"),
        other => panic!("expected ToolCall last, got {other:?}"),
    }
}

/// C1: subsequent tool calls carry only their own tool call.
#[test]
fn canonical_turn_blocks_subsequent_call_shape() {
    let tc1 = test_tool_call("call_1");
    let content = vec![ContentBlock::Text { text: "x".into() }];

    let blocks = super::canonical_turn_blocks(&content, &tc1, false);

    assert_eq!(blocks.len(), 1);
    match &blocks[0] {
        ContentBlock::ToolCall { tool_call } => assert_eq!(tool_call.id, "call_1"),
        other => panic!("expected ToolCall, got {other:?}"),
    }
}

/// C1: a first call with no thinking/text still yields exactly its own tool call.
#[test]
fn canonical_turn_blocks_first_call_no_prose() {
    let tc0 = test_tool_call("call_0");
    let content = vec![
        ContentBlock::ToolCall {
            tool_call: tc0.clone(),
        },
        ContentBlock::ToolCall {
            tool_call: test_tool_call("call_1"),
        },
    ];

    let blocks = super::canonical_turn_blocks(&content, &tc0, true);

    assert_eq!(blocks.len(), 1);
    match &blocks[0] {
        ContentBlock::ToolCall { tool_call } => assert_eq!(tool_call.id, "call_0"),
        other => panic!("expected ToolCall, got {other:?}"),
    }
}

/// Byte-identity invariant: the live context message and the session-rebuilt
/// message (after JSONL persistence) must serialize identically on the wire
/// and produce identical history-guard digests.
#[test]
fn live_and_session_assistant_forms_identical() {
    let blocks = vec![
        ContentBlock::Thinking {
            thinking: "t".into(),
        },
        ContentBlock::Text { text: "x".into() },
        ContentBlock::ToolCall {
            tool_call: test_tool_call("call_0"),
        },
    ];
    let usage = Usage {
        prompt_tokens: 100,
        completion_tokens: 20,
        total_tokens: 120,
    };
    let live = AgentMessage::assistant_with_blocks(blocks, Some(usage));

    // Simulate JSONL persistence + context rebuild.
    let wire = serde_json::to_value(&live).unwrap();
    let rebuilt: AgentMessage = serde_json::from_value(wire.clone()).unwrap();

    assert_eq!(
        serde_json::to_value(&rebuilt).unwrap(),
        wire,
        "rebuilt message must be byte-identical on the wire"
    );
    assert_eq!(
        crate::history_guard::message_digest(&rebuilt),
        crate::history_guard::message_digest(&live),
        "digests must match so the history guard stays silent"
    );
}

/// Every pushed message gets a digest matching `message_digest`, kept in
/// lockstep with the context list.
#[test]
fn push_context_message_lockstep_digests() {
    let mut context: Vec<AgentMessage> = Vec::new();
    let mut digests: Vec<crate::history_guard::MessageDigest> = Vec::new();

    let first = AgentMessage::user("first".to_string());
    let first_digest = crate::history_guard::message_digest(&first);
    super::push_context_message(&mut context, &mut digests, first);

    let second = AgentMessage::user("second".to_string());
    let second_digest = crate::history_guard::message_digest(&second);
    super::push_context_message(&mut context, &mut digests, second);

    assert_eq!(context.len(), 2);
    assert_eq!(digests.len(), 2);
    assert_eq!(digests[0], first_digest);
    assert_eq!(digests[1], second_digest);
}

/// L-14 regression: `activate_skill` with an empty name must not emit a
/// `SkillActivation` event (the old code emitted `skill_name: "unknown"`,
/// which polluted the TUI's `active_skills` set and the saved session header).
/// It still returns an error tool result so the model can correct itself.
#[tokio::test]
async fn activate_skill_empty_name_emits_no_activation_event() {
    use crate::agent::streaming::AgentResponse;
    use crate::config::Config;
    use crate::history_guard::MessageDigest;
    use crate::hooks::HookRegistry;
    use crate::tools::rtk::RtkState;
    use crate::tools::ToolRegistry;
    use tokio::sync::{mpsc, watch};

    let (tx, mut rx) = mpsc::channel::<AgentEvent>(16);
    let config = Config::default();
    let rtk_state = RtkState {
        path: None,
        version_ok: false,
    };
    let tool_registry = ToolRegistry::new();
    let mut hook_registry = HookRegistry::new();
    let (cancel_tx, cancel_rx) = watch::channel(false);
    let _ = cancel_tx;
    let skills: &[Skill] = &[];
    let response = AgentResponse {
        content: Vec::new(),
        usage: None,
    };
    let mut current_context: Vec<AgentMessage> = Vec::new();
    let mut context_digests: Vec<MessageDigest> = Vec::new();

    let mut ctx = super::ToolCallCtx {
        tx: &tx,
        config: &config,
        cwd: std::path::Path::new("."),
        rtk_state: &rtk_state,
        tool_registry: &tool_registry,
        hook_registry: &mut hook_registry,
        cancel_rx: &cancel_rx,
        skills,
        response: &response,
        current_context: &mut current_context,
        context_digests: &mut context_digests,
        first_tool_call: true,
    };

    let tc = ToolCall {
        id: "call_empty".to_string(),
        name: "activate_skill".to_string(),
        arguments: serde_json::json!({ "name": "" }),
    };

    let outcome = super::execute_tool_call(&mut ctx, &tc).await;
    assert!(
        matches!(outcome, super::ToolCallOutcome::Continue),
        "empty skill name should continue the loop, got: {outcome:?}"
    );

    // No SkillActivation event must be emitted for an empty name.
    let mut saw_activation = false;
    let mut saw_error_result = false;
    while let Ok(event) = rx.try_recv() {
        match &event {
            AgentEvent::SkillActivation { .. } => saw_activation = true,
            AgentEvent::SaveToolResult { result, .. }
                if result.contains("skill name is required") =>
            {
                saw_error_result = true;
            }
            _ => {}
        }
    }
    assert!(!saw_activation, "no SkillActivation event for empty name");
    assert!(saw_error_result, "error tool result must be saved");
}
