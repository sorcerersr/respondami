//! Tests for `HistoryGuard` and `MessageDigest` — request-time history
//! prefix stability.

use crate::history_guard::{HistoryGuard, MessageDigest, message_digest};
use crate::image::ImageContent;
use crate::session::{AgentMessage, ContentBlock, ToolCall, Usage};

fn image_tool_result(id: &str, result: &str) -> AgentMessage {
    let img = ImageContent {
        data: "aW1hZ2U=".to_string(),
        mime_type: "image/png".to_string(),
        width: 100,
        height: 50,
    };
    AgentMessage::tool(id.to_string(), "read".to_string(), serde_json::json!({}), result.to_string(), vec![img])
}

fn assistant_with(text: &str, usage: bool) -> AgentMessage {
    AgentMessage::assistant_with_blocks(
        vec![ContentBlock::Text { text: text.to_string() }],
        usage.then_some(Usage {
            prompt_tokens: 10,
            completion_tokens: 5,
            total_tokens: 15,
        }),
    )
}

fn user(msg: &str) -> AgentMessage {
    AgentMessage::user(msg.to_string())
}

fn system(content: &str) -> AgentMessage {
    AgentMessage::system(content.to_string())
}

fn tool_result(id: &str, result: &str) -> AgentMessage {
    AgentMessage::tool(id.to_string(), "read".to_string(), serde_json::json!({}), result.to_string(), Vec::new())
}

fn digests(msgs: &[AgentMessage]) -> Vec<MessageDigest> {
    msgs.iter().map(message_digest).collect()
}

// ---------------------------------------------------------------------------
// MessageDigest
// ---------------------------------------------------------------------------

#[test]
fn digest_identical_messages_match() {
    let a = user("hello");
    let b = user("hello");
    assert_eq!(message_digest(&a), message_digest(&b));
}

#[test]
fn digest_tool_with_images_is_stable() {
    // The base64 payload is frozen at ingestion, so a tool message carrying an
    // image must produce an identical digest before and after a JSONL rebuild
    // (the byte-prefix that vLLM prefix caching depends on).
    let a = image_tool_result("t1", "Image: x (100x50, image/png)");
    let wire = serde_json::to_value(&a).unwrap();
    let rebuilt: AgentMessage = serde_json::from_value(wire).unwrap();
    assert_eq!(
        message_digest(&a),
        message_digest(&rebuilt),
        "image tool message must be byte-stable across persistence"
    );
}

#[test]
fn digest_tool_with_images_differs_from_text_only() {
    // A tool message with an image must digest differently from the same
    // message without the image (the image is part of the wire bytes).
    let with_img = image_tool_result("t1", "Image: x");
    let without_img = tool_result("t1", "Image: x");
    assert_ne!(message_digest(&with_img), message_digest(&without_img));
}

#[test]
fn digest_differs_when_content_changes() {
    let a = user("hello");
    let b = user("world");
    assert_ne!(message_digest(&a), message_digest(&b));
}

#[test]
fn digest_usage_is_stripped_from_assistant_messages() {
    // Usage is never sent to the LLM, so a usage-only change must not
    // change the digest — that is not a cache break.
    let with_usage = assistant_with("result", true);
    let without_usage = assistant_with("result", false);
    assert_eq!(message_digest(&with_usage), message_digest(&without_usage));
}

#[test]
fn digest_differs_when_assistant_content_changes() {
    let a = assistant_with("result", true);
    let b = assistant_with("different", true);
    assert_ne!(message_digest(&a), message_digest(&b));
}

#[test]
fn digest_fnv1a_known_value() {
    // FNV-1a 64-bit test vectors: empty input is the offset basis, "a" is
    // the canonical single-char value. Also pins the hex Display format.
    assert_eq!(MessageDigest::from_bytes(b"").to_string(), "0xcbf29ce484222325");
    assert_eq!(MessageDigest::from_bytes(b"a").to_string(), "0xaf63dc4c8601ec8c");
}

// ---------------------------------------------------------------------------
// HistoryGuard — prefix checks
// ---------------------------------------------------------------------------

#[test]
fn guard_first_request_is_always_ok() {
    let mut guard = HistoryGuard::new();
    let first = digests(&[user("q1")]);
    assert!(guard.check_and_update(&first).is_none());
}

#[test]
fn guard_pure_append_is_ok() {
    let mut guard = HistoryGuard::new();
    let req1 = digests(&[user("q1"), assistant_with("a1", false), tool_result("t1", "r1")]);
    assert!(guard.check_and_update(&req1).is_none());

    // Next request appends more — same prefix, so no violation.
    let mut req2 = req1.clone();
    req2.push(message_digest(&assistant_with("a2", false)));
    req2.push(message_digest(&user("q2")));
    assert!(guard.check_and_update(&req2).is_none());
}

#[test]
fn guard_identical_resend_is_ok_retry() {
    // A retried request re-sends the identical context — this must NOT warn.
    let mut guard = HistoryGuard::new();
    let req1 = digests(&[user("q1"), assistant_with("a1", false)]);
    assert!(guard.check_and_update(&req1).is_none());
    assert!(guard.check_and_update(&req1).is_none(), "identical resend is a retry, not a violation");
}

#[test]
fn guard_mid_list_divergence_is_violation() {
    let mut guard = HistoryGuard::new();
    // Previous: user "q1" then assistant "a1".
    let req1 = digests(&[user("q1"), assistant_with("a1", false)]);
    assert!(guard.check_and_update(&req1).is_none());

    // Current: same first message, but the second message changed (the
    // reported red cell: assistant message re-serialized differently).
    let req2 = digests(&[user("q1"), assistant_with("a1-REWRITTEN", false)]);
    let violation = guard.check_and_update(&req2);
    assert!(violation.is_some(), "mid-list divergence must be a violation");
    let v = violation.unwrap();
    assert!(!v.shrank);
    assert_eq!(v.index, 1, "divergence reported at the second message");
    assert_ne!(v.previous, v.current);
}

#[test]
fn guard_prefix_replaced_is_violation_at_index_zero() {
    // The system prompt (index 0) changed — e.g. a legitimate cache break.
    let mut guard = HistoryGuard::new();
    let req1 = digests(&[system("v1"), user("q1")]);
    assert!(guard.check_and_update(&req1).is_none());

    let req2 = digests(&[system("v2-changed"), user("q1")]);
    let v = guard.check_and_update(&req2).unwrap();
    assert_eq!(v.index, 0);
}

#[test]
fn guard_shrink_is_violation() {
    let mut guard = HistoryGuard::new();
    let req1 = digests(&[user("q1"), assistant_with("a1", false), tool_result("t1", "r1")]);
    assert!(guard.check_and_update(&req1).is_none());

    // Current is shorter than previous — history shrank without a reset.
    let req2 = digests(&[user("q1")]);
    let v = guard.check_and_update(&req2).unwrap();
    assert!(v.shrank, "shrink must be reported as a violation");
    assert_eq!(v.index, 1);
}

#[test]
fn guard_reset_clears_baseline() {
    let mut guard = HistoryGuard::new();
    let req1 = digests(&[user("q1"), assistant_with("a1", false)]);
    assert!(guard.check_and_update(&req1).is_none());

    // Legitimate rewrite (session resume / compaction) — reset, then a
    // wholly new history is fine.
    guard.reset();
    let fresh = digests(&[user("totally-new")]);
    assert!(guard.check_and_update(&fresh).is_none());
}

// ---------------------------------------------------------------------------
// Simulated live-vs-resume scenario (the reported C_N/C_N+1 case)
// ---------------------------------------------------------------------------

#[test]
fn guard_live_vs_resume_divergence() {
    // Live context builds a reduced assistant message for a tool turn:
    //   [system, user, assistant(tool-only)]
    let live: Vec<MessageDigest> = vec![
        message_digest(&system("sys")),
        message_digest(&user("q1")),
        // Reduced: just the tool call, no text/thinking.
        message_digest(&AgentMessage::assistant_with_blocks(
            vec![ContentBlock::ToolCall {
                tool_call: ToolCall {
                    id: "t1".to_string(),
                    name: "read".to_string(),
                    arguments: serde_json::json!({"path": "src/main.rs"}),
                },
            }],
            None,
        )),
        message_digest(&tool_result("t1", "file contents")),
    ];
    let mut guard = HistoryGuard::new();
    assert!(guard.check_and_update(&live).is_none());

    // After the turn, the next request rebuilds its prefix from the session,
    // where the assistant message is the FULL form (text + thinking + tool
    // call). The prefix at the assistant position no longer matches → the
    // reported red cell.
    let resumed: Vec<MessageDigest> = vec![
        message_digest(&system("sys")),
        message_digest(&user("q1")),
        message_digest(&AgentMessage::assistant_with_blocks(
            vec![
                ContentBlock::Thinking { thinking: "thinking...".to_string() },
                ContentBlock::Text { text: "reading the file".to_string() },
                ContentBlock::ToolCall {
                    tool_call: ToolCall {
                        id: "t1".to_string(),
                        name: "read".to_string(),
                        arguments: serde_json::json!({"path": "src/main.rs"}),
                    },
                },
            ],
            Some(Usage {
                prompt_tokens: 100,
                completion_tokens: 20,
                total_tokens: 120,
            }),
        )),
        message_digest(&tool_result("t1", "file contents")),
    ];
    let v = guard.check_and_update(&resumed).unwrap();
    assert_eq!(v.index, 2, "divergence at the assistant message position");
}

#[test]
fn guard_canonical_shape_is_silent() {
    // With the canonical construction, live and session hold byte-identical
    // assistant messages, so the resumed prefix matches and the guard is
    // silent.
    let canonical_assistant = AgentMessage::assistant_with_blocks(
        vec![
            ContentBlock::Thinking { thinking: "thinking...".to_string() },
            ContentBlock::Text { text: "reading the file".to_string() },
            ContentBlock::ToolCall {
                tool_call: ToolCall {
                    id: "t1".to_string(),
                    name: "read".to_string(),
                    arguments: serde_json::json!({"path": "src/main.rs"}),
                },
            },
        ],
        Some(Usage {
            prompt_tokens: 100,
            completion_tokens: 20,
            total_tokens: 120,
        }),
    );
    let same = AgentMessage::assistant_with_blocks(
        vec![
            ContentBlock::Thinking { thinking: "thinking...".to_string() },
            ContentBlock::Text { text: "reading the file".to_string() },
            ContentBlock::ToolCall {
                tool_call: ToolCall {
                    id: "t1".to_string(),
                    name: "read".to_string(),
                    arguments: serde_json::json!({"path": "src/main.rs"}),
                },
            },
        ],
        Some(Usage {
            prompt_tokens: 100,
            completion_tokens: 20,
            total_tokens: 120,
        }),
    );
    assert_eq!(message_digest(&canonical_assistant), message_digest(&same));

    let mut guard = HistoryGuard::new();
    let first: Vec<MessageDigest> = vec![
        message_digest(&system("sys")),
        message_digest(&user("q1")),
        message_digest(&canonical_assistant),
    ];
    assert!(guard.check_and_update(&first).is_none());
    let second: Vec<MessageDigest> = vec![
        message_digest(&system("sys")),
        message_digest(&user("q1")),
        message_digest(&same),
        message_digest(&tool_result("t1", "file contents")),
    ];
    assert!(guard.check_and_update(&second).is_none(), "byte-identical prefix must be silent");
}

// ---- Skill-hook context injection (G2) ----

/// Skill-hook injections (`UserPromptSubmit` → user message, `PreToolUse` → tool
/// result) are persisted consistently: the model-bound form is what is both
/// sent and saved, so the guard must stay silent across consecutive requests.
#[test]
fn guard_hook_injected_prefix_is_stable() {
    // The model-bound forms: hook stdout appended before send and save.
    let injected_user = user("hello\n\n[hook: ups.sh]\ninjected-context");
    let injected_tool = tool_result("t1", "file written\n\n[hook: planning:inject-plan.sh]\nkeep the plan");

    // Simulate JSONL persistence + context rebuild: the rebuilt messages must
    // be byte-identical on the wire, so digests match.
    let rebuilt_user: AgentMessage =
        serde_json::from_value(serde_json::to_value(&injected_user).unwrap()).unwrap();
    let rebuilt_tool: AgentMessage =
        serde_json::from_value(serde_json::to_value(&injected_tool).unwrap()).unwrap();
    assert_eq!(
        message_digest(&rebuilt_user),
        message_digest(&injected_user),
        "persisted user message must digest identically"
    );
    assert_eq!(
        message_digest(&rebuilt_tool),
        message_digest(&injected_tool),
        "persisted tool result must digest identically"
    );

    let mut guard = HistoryGuard::new();
    // Request 1: system + hook-injected user message.
    let req1: Vec<MessageDigest> = vec![
        message_digest(&system("sys")),
        message_digest(&injected_user),
    ];
    assert!(guard.check_and_update(&req1).is_none());
    // Request 2: rebuilt prefix + assistant + hook-injected tool result.
    let req2: Vec<MessageDigest> = vec![
        message_digest(&system("sys")),
        message_digest(&rebuilt_user),
        message_digest(&assistant_with("a1", false)),
        message_digest(&rebuilt_tool),
    ];
    assert!(
        guard.check_and_update(&req2).is_none(),
        "hook-injected, consistently persisted prefix must be silent"
    );
}

/// The counter-case: if the live context carried the clean user message while
/// the session stored the hook-injected one (or vice versa), the guard must
/// flag the drift — proving the previous test exercises the invariant.
#[test]
fn guard_detects_hook_injection_divergence() {
    let clean_user = user("hello");
    let injected_user = user("hello\n\n[hook: ups.sh]\ninjected-context");

    let mut guard = HistoryGuard::new();
    let req1: Vec<MessageDigest> = vec![
        message_digest(&system("sys")),
        message_digest(&injected_user),
    ];
    assert!(guard.check_and_update(&req1).is_none());
    let req2: Vec<MessageDigest> = vec![
        message_digest(&system("sys")),
        message_digest(&clean_user),
        message_digest(&assistant_with("a1", false)),
    ];
    assert!(
        guard.check_and_update(&req2).is_some(),
        "live/persisted divergence at the injected message must be detected"
    );
}
