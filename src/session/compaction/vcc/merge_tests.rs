//! Tests for VCC summary merging.

use super::merge::merge_previous;

#[test]
fn merge_no_previous_returns_fresh() {
    let fresh = "[Session Goal]\n- do the thing";
    let merged = merge_previous("", fresh, 120);
    assert_eq!(merged, fresh);
}

#[test]
fn merge_outstanding_context_uses_fresh() {
    let prev = "[Outstanding Context]\n- old blocker";
    let fresh = "[Outstanding Context]\n- new blocker";
    let merged = merge_previous(prev, fresh, 120);
    assert!(merged.contains("new blocker"));
    assert!(!merged.contains("old blocker"));
}

#[test]
fn merge_dedups_session_goal() {
    let prev = "[Session Goal]\n- goal A\n- goal B";
    let fresh = "[Session Goal]\n- goal B\n- goal C";
    let merged = merge_previous(prev, fresh, 120);
    // goal B appears once.
    assert_eq!(merged.matches("goal B").count(), 1);
    assert!(merged.contains("goal A"));
    assert!(merged.contains("goal C"));
}

#[test]
fn merge_files_by_category() {
    let prev = "[Files And Changes]\n- Modified: a.rs";
    let fresh = "[Files And Changes]\n- Modified: b.rs";
    let merged = merge_previous(prev, fresh, 120);
    assert!(merged.contains("a.rs"));
    assert!(merged.contains("b.rs"));
}

#[test]
fn merge_brief_concatenates() {
    let prev = "[Session Goal]\n- goal\n\n---\n\n[user] old line";
    let fresh = "[Session Goal]\n- goal\n\n---\n\n[user] new line";
    let merged = merge_previous(prev, fresh, 120);
    assert!(merged.contains("old line"));
    assert!(merged.contains("new line"));
}

#[test]
fn merge_caps_brief_lines() {
    let prev_brief = (0..200).map(|i| format!("[user] prev {i}")).collect::<Vec<_>>().join("\n");
    let prev = format!("[Session Goal]\n- goal\n\n---\n\n{prev_brief}");
    let fresh = "[Session Goal]\n- goal\n\n---\n\n[user] new line";
    let merged = merge_previous(&prev, fresh, 120);
    // The merged brief should be capped.
    let lines: Vec<&str> = merged.lines().collect();
    assert!(lines.len() <= 130);
}

#[test]
fn merge_strips_stale_skills_block() {
    // A previous summary carries an <active-skills> block (appended after the
    // brief separator). The merge must drop it; `compute()` re-appends a fresh
    // one, so the final summary has exactly one block.
    let prev = "[Session Goal]\n- goal\n\n---\n\n[user] old line\n\n<active-skills>\nOLD\n</active-skills>";
    let fresh = "[Session Goal]\n- goal\n\n---\n\n[user] new line";
    let merged = merge_previous(prev, fresh, 120);
    let full = format!("{merged}\n\n<active-skills>\nNEW\n</active-skills>");
    assert_eq!(full.matches("<active-skills>").count(), 1);
    assert!(!full.contains("OLD"));
    assert!(full.contains("NEW"));
}

#[test]
fn merge_strips_truncated_skills_block() {
    // A block truncated by the brief cap has no closing tag — strip to EOL.
    let prev = "[Session Goal]\n- goal\n\n---\n\n[user] old line\n\n<active-skills>\nOLD TRUNCATED";
    let fresh = "[Session Goal]\n- goal\n\n---\n\n[user] new line";
    let merged = merge_previous(prev, fresh, 120);
    assert!(!merged.contains("<active-skills>"));
    assert!(!merged.contains("OLD TRUNCATED"));
    assert!(merged.contains("new line"));
}
