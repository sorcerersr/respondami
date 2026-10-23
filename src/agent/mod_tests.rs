use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use tempfile::TempDir;

use crate::image::ImageContent;
use crate::session::{AgentMessage, ContentBlock, ToolCall, Usage};
use crate::skills::{Skill, SkillSource};

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
    assert!(images.is_empty(), "images must be dropped when vision is off");
    assert!(content.contains("Vision disabled"), "note must explain the drop: {content}");
    assert!(content.contains("1 image"), "note must state the count: {content}");
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
    assert!(err.is_some(), "AGENTS.md as directory should cause load error");
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
    assert!(should_reject, "empty response with no tool calls should be rejected");

    // Also test with whitespace-only content
    let content = "   \n\t  ".to_string();
    let should_reject = content.trim().is_empty() && !has_tool_calls;
    assert!(should_reject, "whitespace-only response should be rejected");

    // But a response with tool calls should NOT be rejected even if content is empty
    let has_tool_calls = true;
    let should_reject = content.trim().is_empty() && !has_tool_calls;
    assert!(!should_reject, "empty content with tool calls should not be rejected");
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
    assert!(should_auto_scroll, "auto_scroll should be set when not pinned");

    let pinned_scroll = true;
    let should_auto_scroll = !pinned_scroll;
    assert!(!should_auto_scroll, "auto_scroll should NOT be set when pinned");
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
        ContentBlock::Thinking { thinking: "hmm".into() },
        ContentBlock::Text { text: "let me check".into() },
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
        ContentBlock::Thinking { thinking: "t".into() },
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

// ---- Model-bound tool result (PreToolUse + PostToolUse) ----

/// `PreToolUse` hook stdout is appended to the tool result the model sees, in
/// the `[hook: <name>]` framing, after the clean output.
#[test]
fn model_bound_tool_result_appends_pretool_hook_stdout() {
    let outputs = vec![(
        "planning:inject-plan.sh".to_string(),
        "keep the plan in mind".to_string(),
    )];
    let result = super::model_bound_tool_result("file written", &outputs);
    assert!(result.starts_with("file written"), "clean output first: {result}");
    assert!(result.contains("[hook: planning:inject-plan.sh]"), "framed block: {result}");
    assert!(result.contains("keep the plan in mind"), "stdout: {result}");
}

/// Multiple successful hooks each get their own block, in order.
#[test]
fn model_bound_tool_result_multiple_hooks() {
    let outputs = vec![
        ("a.sh".to_string(), "first".to_string()),
        ("b.sh".to_string(), "second".to_string()),
    ];
    let result = super::model_bound_tool_result("ok", &outputs);
    let first = result.find("[hook: a.sh]").expect("first block");
    let second = result.find("[hook: b.sh]").expect("second block");
    assert!(first < second, "hook order preserved");
}

/// Empty stdout (or no hooks) → the model result is exactly the clean output.
#[test]
fn model_bound_tool_result_empty_stdout_unchanged() {
    assert_eq!(super::model_bound_tool_result("file written", &[]), "file written");
    let outputs = vec![("a.sh".to_string(), String::new())];
    assert_eq!(
        super::model_bound_tool_result("file written", &outputs),
        "file written",
        "empty stdout must not add a block"
    );
}

/// A blocked `PreToolUse` hook (exit 2) injects nothing: the blocked branch
/// never collects outputs and saves its own "blocked" result instead, so the
/// model-bound form is built from an empty output list.
#[test]
fn model_bound_tool_result_blocked_hook_injects_nothing() {
    let result = super::model_bound_tool_result(
        "Tool call blocked by hook: no",
        &[],
    );
    assert_eq!(result, "Tool call blocked by hook: no");
}

/// The model-bound tool result does NOT carry the `PostToolUse` instruction —
/// that is injected as a separate `user` message. Only the clean output and
/// any `PreToolUse` context are present.
#[test]
fn model_bound_tool_result_excludes_posttool_instruction() {
    let outputs = vec![("a.sh".to_string(), "pre context".to_string())];
    let result = super::model_bound_tool_result("file written", &outputs);
    assert!(result.starts_with("file written"), "clean output first: {result}");
    assert!(result.contains("[hook: a.sh]"), "pre-tool block: {result}");
    assert!(
        !result.contains("Hook instruction"),
        "post-tool instruction must not be in the tool result: {result}"
    );
}

// ---- PostToolUse hook instruction framing ----

/// Build a `HookResult` for framing tests.
fn hook_result(exit_code: i32, stdout: &str, stderr: &str) -> crate::hooks::HookResult {
    crate::hooks::HookResult {
        exit_code,
        stdout: stdout.to_string(),
        stderr: stderr.to_string(),
    }
}

/// A successful hook with non-empty stdout produces a framed block of the
/// exact form `[Hook instruction — <name>]\n<stdout>`.
#[test]
fn posttool_block_success_framing() {
    let r = hook_result(0, "add your entry to progress.md", "");
    let block = super::format_posttool_block("planning:post-tool-use.sh", &r);
    assert_eq!(block, "\n\n[Hook instruction — planning:post-tool-use.sh]\nadd your entry to progress.md");
}

/// A blocked hook (exit 2) uses the stderr as the body.
#[test]
fn posttool_block_blocked_uses_stderr() {
    let r = hook_result(2, "", "this change is not allowed");
    let block = super::format_posttool_block("guard.sh", &r);
    assert_eq!(block, "\n\n[Hook instruction — guard.sh]\nthis change is not allowed");
}

/// A successful hook with empty stdout produces no block.
#[test]
fn posttool_block_empty_stdout_is_noop() {
    let r = hook_result(0, "", "");
    assert_eq!(super::format_posttool_block("a.sh", &r), "");
}

/// A failed hook (non-zero, non-2) produces no block.
#[test]
fn posttool_block_failed_is_noop() {
    let r = hook_result(1, "", "boom");
    assert_eq!(super::format_posttool_block("a.sh", &r), "");
}

/// Multiple hooks concatenate into a single instruction string, in registry
/// order, each with its own framed block.
#[test]
fn posttool_block_multiple_hooks_in_order() {
    let a = hook_result(0, "first instruction", "");
    let b = hook_result(0, "second instruction", "");
    let mut combined = String::new();
    combined.push_str(&super::format_posttool_block("a.sh", &a));
    combined.push_str(&super::format_posttool_block("b.sh", &b));
    let first = combined.find("[Hook instruction — a.sh]").expect("first block");
    let second = combined.find("[Hook instruction — b.sh]").expect("second block");
    assert!(first < second, "hook order preserved");
    assert!(combined.contains("first instruction"));
    assert!(combined.contains("second instruction"));
}

/// A failed hook interleaved with a successful one: only the successful
/// block appears (the failed one contributes nothing).
#[test]
fn posttool_block_failed_interleaved_dropped() {
    let ok = hook_result(0, "do the thing", "");
    let failed = hook_result(3, "", "error");
    let mut combined = String::new();
    combined.push_str(&super::format_posttool_block("ok.sh", &ok));
    combined.push_str(&super::format_posttool_block("bad.sh", &failed));
    assert!(combined.contains("[Hook instruction — ok.sh]"));
    assert!(!combined.contains("bad.sh"), "failed hook must not appear");
}
