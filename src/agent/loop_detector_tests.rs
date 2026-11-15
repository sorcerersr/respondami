//! Tests for [`LoopDetector`] — degenerate loop detection.

use serde_json::json;

use crate::agent::loop_detector::{
    LoopDetector, canonical_args, loop_recovery_prompt, preview_args,
};

/// Build a detector with the default window/threshold.
fn detector() -> LoopDetector {
    LoopDetector::new(20, 8)
}

/// Record `n` identical `bash` calls.
fn record_n_bash(det: &mut LoopDetector, n: usize) {
    for _ in 0..n {
        det.record("bash", &json!({"command": "grep -n foo src/lib.rs"}));
    }
}

// ---------------------------------------------------------------------------
// canonical_args
// ---------------------------------------------------------------------------

#[test]
fn canonical_args_key_order_invariant() {
    // {a:1,b:2} and {b:2,a:1} must canonicalize identically (BTreeMap sort).
    assert_eq!(
        canonical_args(&json!({"a": 1, "b": 2})),
        canonical_args(&json!({"b": 2, "a": 1}))
    );
}

#[test]
fn canonical_args_differs_on_value() {
    assert_ne!(
        canonical_args(&json!({"command": "ls"})),
        canonical_args(&json!({"command": "ls -la"}))
    );
}

#[test]
fn canonical_args_nested_objects_sorted() {
    assert_eq!(
        canonical_args(&json!({"z": {"b": 1, "a": 2}, "a": 0})),
        canonical_args(&json!({"a": 0, "z": {"a": 2, "b": 1}}))
    );
}

// ---------------------------------------------------------------------------
// preview_args
// ---------------------------------------------------------------------------

#[test]
fn preview_args_short_unchanged() {
    assert_eq!(preview_args(r#"{"command":"ls"}"#), r#"{"command":"ls"}"#);
}

#[test]
fn preview_args_truncates_long() {
    let long = format!("{{\"command\":\"{}\"}}", "x".repeat(200));
    let preview = preview_args(&long);
    assert!(preview.chars().count() <= 81, "preview must be truncated");
    assert!(preview.ends_with('…'), "truncated preview ends with ellipsis");
}

#[test]
fn preview_args_collapses_newlines() {
    let multiline = r#"{"command":"echo a\necho b"}"#;
    let preview = preview_args(multiline);
    assert!(!preview.contains('\n'), "preview must be single-line");
}

// ---------------------------------------------------------------------------
// detect — threshold behavior
// ---------------------------------------------------------------------------

#[test]
fn detect_below_threshold_none() {
    let mut det = detector();
    for _ in 0..7 {
        det.record("bash", &json!({"command": "grep -n foo"}));
    }
    assert!(det.detect().is_none(), "7 < threshold 8 must not trip");
}

#[test]
fn detect_at_threshold_some() {
    let mut det = detector();
    record_n_bash(&mut det, 8);
    let report = det.detect().expect("8 == threshold must trip");
    assert_eq!(report.tool_name, "bash");
    assert_eq!(report.count, 8);
}

#[test]
fn detect_above_threshold_reports_full_count() {
    let mut det = detector();
    record_n_bash(&mut det, 12);
    let report = det.detect().expect("12 > threshold must trip");
    assert_eq!(report.count, 12);
}

#[test]
fn detect_distinct_calls_never_trip() {
    // 20 different commands — no single key reaches the threshold.
    let mut det = detector();
    for i in 0..20 {
        det.record("bash", &json!({"command": format!("cmd-{i}") }));
    }
    assert!(det.detect().is_none(), "distinct calls must not trip");
}

#[test]
fn detect_tool_alternation_still_trips_on_same_key() {
    // Alternating two tools, each 10× — the window holds 10 bash + 10 read;
    // each same-key count is 10 ≥ 8. Detection is per-key frequency, so it
    // trips even though the calls alternate between tools.
    let mut det = detector();
    for _ in 0..10 {
        det.record("bash", &json!({"command": "grep -n foo"}));
        det.record("read", &json!({"path": "src/lib.rs"}));
    }
    assert!(det.detect().is_some(), "10 identical calls in a 20-window trips");
}

// ---------------------------------------------------------------------------
// detect — window eviction
// ---------------------------------------------------------------------------

#[test]
fn detect_window_eviction_prevents_stale_trip() {
    // 8 identical calls, then 20 distinct calls evict them from the window.
    let mut det = detector();
    record_n_bash(&mut det, 8);
    assert!(det.detect().is_some(), "trips before eviction");
    for i in 0..20 {
        det.record("bash", &json!({"command": format!("other-{i}") }));
    }
    assert!(
        det.detect().is_none(),
        "evicted repetitions must not keep tripping"
    );
}

#[test]
fn detect_window_bounded() {
    let mut det = LoopDetector::new(5, 3);
    // 3 identical, then 5 distinct (evict all 3).
    for _ in 0..3 {
        det.record("bash", &json!({"command": "same"}));
    }
    assert!(det.detect().is_some());
    for i in 0..5 {
        det.record("bash", &json!({"command": format!("diff-{i}") }));
    }
    assert!(det.detect().is_none());
}

// ---------------------------------------------------------------------------
// detect — the 55b9a55e interleaved pattern
// ---------------------------------------------------------------------------

#[test]
fn detect_interleaved_housekeeping_still_trips() {
    // The observed loop: ~100 identical grep calls with an `edit progress.md`
    // interleaved every ~16. A consecutive-run check would fail; window
    // frequency must trip.
    let mut det = detector();
    let grep = json!({"command": "cd /home/dev/panzaaa && grep -n input_system_is_enabled src"});
    let edit = json!({"path": "progress.md", "edits": []});
    for _ in 0..7 {
        for _ in 0..16 {
            det.record("bash", &grep);
        }
        det.record("edit", &edit);
    }
    let report = det
        .detect()
        .expect("interleaved identical calls must trip");
    assert_eq!(report.tool_name, "bash");
    assert!(report.count >= 8, "the grep call must be the reported loop");
}

#[test]
fn detect_reports_active_loop_key() {
    // Two keys both reach the threshold; the one with the newest occurrence
    // (the loop the model is in now) is reported.
    let mut det = LoopDetector::new(20, 2);
    for _ in 0..2 {
        det.record("bash", &json!({"command": "old-loop"}));
    }
    for _ in 0..2 {
        det.record("bash", &json!({"command": "new-loop"}));
    }
    let report = det.detect().expect("must trip");
    assert!(
        report.canonical_args.contains("new-loop"),
        "the most recent loop must be reported"
    );
}

// ---------------------------------------------------------------------------
// degenerate construction
// ---------------------------------------------------------------------------

#[test]
fn detector_zero_window_and_threshold_clamped() {
    // window 0 → 1, threshold 0 → 1: a single call trips.
    let mut det = LoopDetector::new(0, 0);
    det.record("bash", &json!({"command": "x"}));
    assert!(det.detect().is_some());
}

// ---------------------------------------------------------------------------
// loop_recovery_prompt
// ---------------------------------------------------------------------------

#[test]
fn recovery_prompt_names_call_and_budget() {
    let prompt = loop_recovery_prompt("bash", "grep -n foo", 12, 1, 3);
    assert!(prompt.contains("bash"), "must name the tool");
    assert!(prompt.contains("grep -n foo"), "must name the repeated call");
    assert!(prompt.contains("12"), "must state the count");
    assert!(prompt.contains("1 of 3"), "must show the recovery budget");
    // It must steer toward a different action, not a repeat.
    assert!(prompt.to_lowercase().contains("different"));
}

#[test]
fn recovery_prompt_is_not_empty_and_single_purpose() {
    let prompt = loop_recovery_prompt("read", "src/lib.rs", 8, 2, 3);
    assert_ne!(prompt.trim(), "");
    assert!(prompt.contains("2 of 3"));
}

// ---------------------------------------------------------------------------
// Regression: session 55b9a55e pattern
// ---------------------------------------------------------------------------

/// Regression test for the real loop in session `55b9a55e`:
/// the same `bash` call (`grep -n "input_system_is_enabled"`) repeated ~100×
/// with an `edit` to `progress.md` interleaved every ~16 iterations. A
/// consecutive-run detector would never trip (the `edit` breaks the run), so
/// this verifies the sliding-window frequency approach catches it.
#[test]
fn regression_55b9a55e_interleaved_housekeeping_trips() {
    let mut det = detector();
    const GREP: &str = r#"grep -n "input_system_is_enabled" src/input.rs"#;
    let mut iterations = 0usize;
    let mut tripped_at: Option<usize> = None;

    // Replay ~40 iterations of the loop: the same grep each time, with an
    // `edit` to progress.md every 16th iteration (the housekeeping that
    // masked the consecutive-run check).
    for i in 0..40 {
        det.record("bash", &json!({"command": GREP}));
        iterations += 1;
        if i % 16 == 15 {
            det.record("edit", &json!({"path": "progress.md", "oldText": "a", "newText": "b"}));
        }
        if tripped_at.is_none() && det.detect().is_some() {
            tripped_at = Some(iterations);
        }
    }

    let tripped_at = tripped_at.expect("the interleaved loop must trip the detector");
    // It trips well before the ~100 iterations the real session ran to, and
    // the report names the grep, not the interleaved edit.
    assert!(tripped_at <= 20, "must trip within the window, got {tripped_at}");
    let report = det.detect().expect("detector must be tripped");
    assert_eq!(report.tool_name, "bash");
    assert!(report.canonical_args.contains("input_system_is_enabled"));
}
