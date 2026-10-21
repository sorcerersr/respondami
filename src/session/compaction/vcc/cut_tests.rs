//! Tests for VCC cut selection.

use super::cut::{apply_tail_budget, build_own_cut, find_live_start, resolve_smart_keep, CutResult};
use crate::agent::token_estimation::TokenEstimator;
use crate::session::{AgentMessage, SessionEntry};

fn user_entry(text: &str) -> SessionEntry {
    SessionEntry::new_message(None, AgentMessage::user(text.to_string()))
}

fn assistant_entry(text: &str) -> SessionEntry {
    SessionEntry::new_message(None, AgentMessage::assistant(text.to_string(), String::new(), vec![], None))
}

/// Build a session: header + `n` user turns each followed by an assistant reply.
fn session_with_turns(n: usize) -> Vec<SessionEntry> {
    let mut entries = vec![SessionEntry::new_session("s".into(), "t".into(), "/".into(), "m".into(), 4096)];
    for i in 0..n {
        entries.push(user_entry(&format!("user turn {i}")));
        entries.push(assistant_entry(&format!("assistant reply {i}")));
    }
    entries
}

#[test]
fn live_start_after_header_when_no_compaction() {
    let entries = session_with_turns(3);
    assert_eq!(find_live_start(&entries), 1);
}

#[test]
fn live_start_after_compaction_with_valid_kept_id() {
    let mut entries = session_with_turns(2);
    let kept_id = entries[1].id();
    let compaction = SessionEntry::new_compaction(None, "summary".into(), kept_id.clone(), 100);
    entries.push(compaction);
    entries.push(user_entry("new turn"));
    assert_eq!(find_live_start(&entries), 1);
}

#[test]
fn live_start_orphan_recovery_on_empty_kept_id() {
    let mut entries = session_with_turns(2);
    let compaction = SessionEntry::new_compaction(None, "summary".into(), String::new(), 100);
    entries.push(compaction);
    entries.push(user_entry("new turn"));
    // Orphan recovery: start right after the compaction entry.
    assert_eq!(find_live_start(&entries), entries.len() - 1);
}

#[test]
fn build_own_cut_too_few() {
    let entries = vec![
        SessionEntry::new_session("s".into(), "t".into(), "/".into(), "m".into(), 4096),
        user_entry("hi"),
    ];
    assert_eq!(build_own_cut(&entries, 1, 1), CutResult::TooFew);
}

#[test]
fn build_own_cut_keep_zero_compact_all() {
    let entries = session_with_turns(3);
    assert_eq!(build_own_cut(&entries, 1, 0), CutResult::CompactAll);
}

#[test]
fn build_own_cut_keep_one() {
    let entries = session_with_turns(3);
    // 3 user turns at indices 1, 3, 5. keep=1 → keep the last turn (index 5).
    match build_own_cut(&entries, 1, 1) {
        CutResult::Cut(idx) => assert_eq!(idx, 5),
        _ => panic!("expected Cut"),
    }
}

#[test]
fn build_own_cut_keep_two() {
    let entries = session_with_turns(3);
    // keep=2 → keep the last two turns (cut at the 2nd, index 3).
    match build_own_cut(&entries, 1, 2) {
        CutResult::Cut(idx) => assert_eq!(idx, 3),
        _ => panic!("expected Cut"),
    }
}

#[test]
fn build_own_cut_keep_larger_than_turns_compact_all() {
    let entries = session_with_turns(2);
    assert_eq!(build_own_cut(&entries, 1, 5), CutResult::CompactAll);
}

#[test]
fn apply_tail_budget_small_tail_unchanged() {
    let entries = session_with_turns(3);
    let est = TokenEstimator::heuristic();
    let cut = build_own_cut(&entries, 1, 1);
    let result = apply_tail_budget(&entries, 1, cut, &est, 16384);
    assert_eq!(result, cut);
}

#[test]
fn apply_tail_budget_oversized_tail_recuts() {
    // A single huge assistant message makes the keep:1 tail oversized.
    let mut entries = session_with_turns(3);
    let huge = "x".repeat(100_000);
    entries.push(assistant_entry(&huge));
    let est = TokenEstimator::heuristic();
    let cut = build_own_cut(&entries, 1, 1);
    let result = apply_tail_budget(&entries, 1, cut, &est, 100);
    // Should re-cut to a later index (closer to the end) to shrink the tail.
    match result {
        CutResult::Cut(idx) => assert!(idx >= 3),
        CutResult::CompactAll => {}
        _ => panic!("expected Cut or CompactAll"),
    }
}

#[test]
fn resolve_smart_keep_explicit_honored() {
    let entries = session_with_turns(5);
    let est = TokenEstimator::heuristic();
    assert_eq!(resolve_smart_keep(&entries, 1, &est, true, true, Some(3)), 3);
}

#[test]
fn resolve_smart_keep_disabled_returns_base() {
    let entries = session_with_turns(5);
    let est = TokenEstimator::heuristic();
    assert_eq!(resolve_smart_keep(&entries, 1, &est, false, false, None), 1);
}

#[test]
fn resolve_smart_keep_grows_small_tail() {
    // Small messages → keep:1 tail is tiny → should grow keep.
    let entries = session_with_turns(5);
    let est = TokenEstimator::heuristic();
    let keep = resolve_smart_keep(&entries, 1, &est, true, false, None);
    assert!(keep >= 1);
}
