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
    let prev_brief = (0..200)
        .map(|i| format!("[user] prev {i}"))
        .collect::<Vec<_>>()
        .join("\n");
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

// ---- Plan-block carry-through (===BEGIN/END PLAN DATA===) ----

const PLAN1: &str = "===BEGIN PLAN DATA===\n# Task Plan: v1\n- phase 1 done\n===END PLAN DATA===";
const PLAN2: &str = "===BEGIN PLAN DATA===\n# Task Plan: v2\n- phase 2 done\n===END PLAN DATA===";

/// 1. A marker block survives a single compaction, verbatim.
#[test]
fn plan_block_survives_single_compaction() {
    let prev = "[Session Goal]\n- goal";
    let fresh = format!("[Session Goal]\n- goal\n\n---\n\n[user] line\n\n{PLAN1}");
    let merged = merge_previous(prev, &fresh, 120);
    assert_eq!(
        merged.matches(PLAN1).count(),
        1,
        "plan block must appear exactly once, verbatim: {merged}"
    );
    assert!(
        merged.ends_with(PLAN1),
        "plan block is re-appended at the end: {merged}"
    );
}

/// 2. A marker block survives two consecutive compactions; the second
///    (updated) block wins.
#[test]
fn plan_block_survives_two_compactions_latest_wins() {
    // First compaction: PLAN1 is carried into the merged summary.
    let first = merge_previous(
        "[Session Goal]\n- goal",
        &format!("[Session Goal]\n- goal\n\n---\n\n[user] line\n\n{PLAN1}"),
        120,
    );
    assert!(first.contains(PLAN1));
    // Second compaction: the fresh summary carries the updated PLAN2; the
    // previous summary carries PLAN1. PLAN2 must win and PLAN1 must be gone.
    let second = merge_previous(
        &first,
        &format!("[Session Goal]\n- goal\n\n---\n\n[user] line2\n\n{PLAN2}"),
        120,
    );
    assert_eq!(
        second.matches(PLAN2).count(),
        1,
        "latest plan block must appear exactly once: {second}"
    );
    assert!(
        !second.contains("Task Plan: v1"),
        "stale plan must be dropped: {second}"
    );
    assert!(second.ends_with(PLAN2));
}

/// 3. Unmarked sections still merge/dedup/cap as before when a plan block is
///    present (the plan block does not perturb the heuristic merge).
#[test]
fn plan_block_does_not_perturb_section_merge() {
    let prev = format!("[Session Goal]\n- goal A\n- goal B\n\n{PLAN1}");
    let fresh = format!("[Session Goal]\n- goal B\n- goal C\n\n{PLAN1}");
    let merged = merge_previous(&prev, &fresh, 120);
    // Section dedup still works: goal B once, A and C present.
    assert_eq!(merged.matches("goal B").count(), 1);
    assert!(merged.contains("goal A"));
    assert!(merged.contains("goal C"));
    // Plan block carried verbatim, exactly once.
    assert_eq!(merged.matches(PLAN1).count(), 1);
}

/// 4. No marker present → behavior is byte-identical to the no-plan path.
#[test]
fn no_plan_block_is_unchanged() {
    let prev = "[Session Goal]\n- goal A";
    let fresh = "[Session Goal]\n- goal A\n\n---\n\n[user] line";
    let with_plan = merge_previous(prev, fresh, 120);
    // A summary that merely contains the marker text as plain prose (not a
    // well-formed block) must be treated as plain text, not carried verbatim.
    let prose = "[Session Goal]\n- goal A\n\n---\n\n[user] saw ===BEGIN PLAN DATA=== once".to_string();
    let merged = merge_previous(prev, &prose, 120);
    assert!(merged.contains("saw ===BEGIN PLAN DATA=== once"));
    assert!(
        !merged.contains("===END PLAN DATA==="),
        "no END marker → not a plan block"
    );
    // Sanity: the plain no-marker merge is stable.
    assert!(with_plan.contains("goal A"));
    assert!(with_plan.contains("[user] line"));
}

/// 5. Malformed marker (BEGIN without END) → treated as plain text (fail-safe):
///    not carried verbatim as its own part, left in the brief where it sits.
#[test]
fn malformed_plan_block_treated_as_plain_text() {
    let malformed = "===BEGIN PLAN DATA===\n# Task Plan: truncated"; // no END
    let prev = "[Session Goal]\n- goal";
    // The malformed block sits in the middle of the brief, with content after
    // it. If it were (wrongly) treated as a plan block it would be moved to
    // the end as its own part; treated as plain text it stays in the brief.
    let fresh =
        format!("[Session Goal]\n- goal\n\n---\n\n[user] line\n\n{malformed}\n\n[user] after");
    let merged = merge_previous(prev, &fresh, 120);
    // The BEGIN marker is present (plain text) but there is no END marker.
    assert!(merged.contains("===BEGIN PLAN DATA==="));
    assert!(!merged.contains("===END PLAN DATA==="));
    // Content after the malformed block is preserved (it stayed in the brief).
    assert!(
        merged.contains("[user] after"),
        "content after the block is kept: {merged}"
    );
    // The malformed block did NOT become its own SEPARATOR-delimited part.
    assert!(
        !merged.contains(&format!("\n\n---\n\n{malformed}")),
        "malformed block must not become its own part: {merged}"
    );
}
