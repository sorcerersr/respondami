//! Tests for `HistoryGuard` and `MessageDigest` — request-time history
//! prefix stability.

use crate::history_guard::{HistoryGuard, MessageDigest, message_digest};
use crate::session::{AgentMessage, ContentBlock, ToolCall, Usage};

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
    AgentMessage::tool(id.to_string(), "read".to_string(), serde_json::json!({}), result.to_string())
}

fn digests(msgs: &[AgentMessage]) -> Vec<Option<MessageDigest>> {
    msgs.iter().map(|m| Some(message_digest(m))).collect()
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
    req2.push(Some(message_digest(&assistant_with("a2", false))));
    req2.push(Some(message_digest(&user("q2"))));
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
// Ephemeral (None) entries
// ---------------------------------------------------------------------------

#[test]
fn guard_ephemeral_entries_are_skipped() {
    let mut guard = HistoryGuard::new();
    // Previous: a persisted message, then an ephemeral hook pair.
    let persisted = Some(message_digest(&user("q1")));
    let ephemeral1 = None;
    let ephemeral2 = None;
    let prev: Vec<Option<MessageDigest>> = vec![persisted, ephemeral1, ephemeral2];
    assert!(guard.check_and_update(&prev).is_none());

    // Current: same persisted message, new ephemeral content in between,
    // plus a fresh persisted append. Ephemeral entries must be ignored, so
    // no violation.
    let current: Vec<Option<MessageDigest>> = vec![
        persisted,
        None, // different ephemeral, still skipped
        Some(message_digest(&user("q2"))),
    ];
    assert!(
        guard.check_and_update(&current).is_none(),
        "ephemeral (None) entries must not cause false positives"
    );
}

#[test]
fn guard_ephemeral_replacement_is_not_a_violation() {
    // The synthetic hook_instruction pair is appended live-only and never
    // persisted. Two consecutive turns that both end in a hook pair must not
    // warn even though the hook content differs.
    let mut guard = HistoryGuard::new();
    let base = Some(message_digest(&assistant_with("a1", false)));
    let hook_turn_1: Vec<Option<MessageDigest>> = vec![base, None, None];
    assert!(guard.check_and_update(&hook_turn_1).is_none());

    let hook_turn_2: Vec<Option<MessageDigest>> = vec![base, None, None];
    assert!(guard.check_and_update(&hook_turn_2).is_none());
}

#[test]
fn guard_divergence_among_persisted_still_detected_with_ephemeral_present() {
    let mut guard = HistoryGuard::new();
    let req1: Vec<Option<MessageDigest>> = vec![
        Some(message_digest(&user("q1"))),
        None,
        Some(message_digest(&assistant_with("a1", false))),
    ];
    assert!(guard.check_and_update(&req1).is_none());

    // The persisted assistant message changed (the real bug) — must still be
    // reported even with ephemeral entries around it.
    let req2: Vec<Option<MessageDigest>> = vec![
        Some(message_digest(&user("q1"))),
        None,
        Some(message_digest(&assistant_with("a1-CHANGED", false))),
    ];
    let v = guard.check_and_update(&req2).unwrap();
    assert!(!v.shrank);
    assert_eq!(v.index, 2, "reported at the raw index of the changed message");
}

// ---------------------------------------------------------------------------
// Simulated live-vs-resume scenario (the reported C_N/C_N+1 case)
// ---------------------------------------------------------------------------

#[test]
fn guard_live_vs_resume_divergence() {
    // Live context builds a reduced assistant message for a tool turn:
    //   [system, user, assistant(tool-only)]
    let live: Vec<Option<MessageDigest>> = vec![
        Some(message_digest(&system("sys"))),
        Some(message_digest(&user("q1"))),
        // Reduced: just the tool call, no text/thinking.
        Some(message_digest(&AgentMessage::assistant_with_blocks(
            vec![ContentBlock::ToolCall {
                tool_call: ToolCall {
                    id: "t1".to_string(),
                    name: "read".to_string(),
                    arguments: serde_json::json!({"path": "src/main.rs"}),
                },
            }],
            None,
        ))),
        Some(message_digest(&tool_result("t1", "file contents"))),
    ];
    let mut guard = HistoryGuard::new();
    assert!(guard.check_and_update(&live).is_none());

    // After the turn, the next request rebuilds its prefix from the session,
    // where the assistant message is the FULL form (text + thinking + tool
    // call). The prefix at the assistant position no longer matches → the
    // reported red cell.
    let resumed: Vec<Option<MessageDigest>> = vec![
        Some(message_digest(&system("sys"))),
        Some(message_digest(&user("q1"))),
        Some(message_digest(&AgentMessage::assistant_with_blocks(
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
        ))),
        Some(message_digest(&tool_result("t1", "file contents"))),
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
    let first: Vec<Option<MessageDigest>> = vec![
        Some(message_digest(&system("sys"))),
        Some(message_digest(&user("q1"))),
        Some(message_digest(&canonical_assistant)),
    ];
    assert!(guard.check_and_update(&first).is_none());
    let second: Vec<Option<MessageDigest>> = vec![
        Some(message_digest(&system("sys"))),
        Some(message_digest(&user("q1"))),
        Some(message_digest(&same)),
        Some(message_digest(&tool_result("t1", "file contents"))),
    ];
    assert!(guard.check_and_update(&second).is_none(), "byte-identical prefix must be silent");
}
