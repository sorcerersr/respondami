//! Tests for [`LoopDetector`] — degenerate loop detection.

use serde_json::json;

use crate::agent::loop_detector::{
    LoopDetector, LoopKind, TextLoopDetector, canonical_args, loop_recovery_prompt, normalize_text,
    preview_args, text_loop_recovery_prompt,
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

// ---------------------------------------------------------------------------
// normalize_text
// ---------------------------------------------------------------------------

#[test]
fn normalize_text_trims_and_collapses_whitespace() {
    assert_eq!(normalize_text("  a\n\n  b  "), "a b");
    assert_eq!(normalize_text("a\t\tb"), "a b");
    assert_eq!(normalize_text(""), "");
    assert_eq!(normalize_text("   "), "");
}

#[test]
fn normalize_text_multiline_matches_single_line() {
    // The observed loop's response spanned multiple lines; equivalent text
    // with different whitespace must produce the same key.
    assert_eq!(
        normalize_text("I need to investigate\nthis further.\nLet me check."),
        normalize_text("I need to investigate this further. Let me check.")
    );
}

// ---------------------------------------------------------------------------
// TextLoopDetector — threshold behavior
// ---------------------------------------------------------------------------

/// Build a text detector with the default window/threshold.
fn text_detector() -> TextLoopDetector {
    TextLoopDetector::new(10, 3)
}

#[test]
fn text_detect_below_threshold_none() {
    let mut det = text_detector();
    assert!(det.record("same text").is_none(), "1 < threshold 3 must not trip");
    assert!(det.record("same text").is_none(), "2 < threshold 3 must not trip");
    assert!(det.record("same text").is_some(), "3 == threshold must trip");
}

#[test]
fn text_detect_at_threshold_some() {
    let mut det = text_detector();
    let _ = det.record("same text");
    let _ = det.record("same text");
    let report = det.record("same text").expect("3 == threshold must trip");
    assert_eq!(report.kind, LoopKind::TextEcho);
    assert_eq!(report.tool_name, "response");
    assert_eq!(report.count, 3);
    assert_eq!(report.canonical_args, "same text");
}

#[test]
fn text_detect_above_threshold_reports_full_count() {
    let mut det = text_detector();
    for _ in 0..5 {
        let _ = det.record("same text");
    }
    let report = det.detect().expect("5 > threshold must trip");
    assert_eq!(report.count, 5);
}

#[test]
fn text_detect_distinct_responses_never_trip() {
    let mut det = text_detector();
    for i in 0..10 {
        let _ = det.record(&format!("response {i}"));
    }
    assert!(det.detect().is_none(), "distinct responses must not trip");
}

#[test]
fn text_detect_empty_not_recorded() {
    let mut det = text_detector();
    for _ in 0..5 {
        let _ = det.record("");
    }
    assert!(det.detect().is_none(), "empty text must not be recorded");
}

#[test]
fn text_detect_normalized_before_comparison() {
    // Whitespace variants of the same text must count as one key.
    let mut det = text_detector();
    let _ = det.record("a  b");
    let _ = det.record("a\n\nb");
    let report = det.record(" a b ").expect("normalized variants must trip");
    assert_eq!(report.count, 3);
}

// ---------------------------------------------------------------------------
// TextLoopDetector — window eviction + tool-activity clear
// ---------------------------------------------------------------------------

#[test]
fn text_detect_window_eviction_prevents_stale_trip() {
    let mut det = text_detector();
    for _ in 0..3 {
        let _ = det.record("looped");
    }
    assert!(det.detect().is_some(), "trips before eviction");
    for i in 0..10 {
        let _ = det.record(&format!("distinct-{i}"));
    }
    assert!(
        det.detect().is_none(),
        "evicted repetitions must not keep tripping"
    );
}

#[test]
fn text_detect_interleaved_repeats_still_trip() {
    // 3 identical within a 10-window, with 7 distinct interleaved — the
    // windowed frequency check must trip (a consecutive-run check would not).
    let mut det = text_detector();
    for i in 0..10 {
        if i % 3 == 0 {
            let _ = det.record("looped");
        } else {
            let _ = det.record(&format!("filler-{i}"));
        }
    }
    assert!(det.detect().is_some(), "3 identical within 10 must trip");
}

#[test]
fn text_note_tool_activity_clears_window() {
    let mut det = text_detector();
    for _ in 0..3 {
        let _ = det.record("looped");
    }
    assert!(det.detect().is_some(), "trips before the tool turn");
    det.note_tool_activity();
    assert!(
        det.detect().is_none(),
        "a tool-call turn is progress and must clear the window"
    );
}

#[test]
fn text_reset_clears_window() {
    let mut det = text_detector();
    for _ in 0..3 {
        let _ = det.record("looped");
    }
    assert!(det.detect().is_some());
    det.reset();
    assert!(det.detect().is_none(), "reset must clear the window");
}

#[test]
fn text_reset_after_recovery_allows_different_response() {
    // Regression (H-1): after a trip + recovery (which resets the window), a
    // *different* response must not re-trip on the stale key. Without the
    // reset, the old key stays ≥ threshold and detect() reports it for any
    // new text, trapping the model in a recovery cycle.
    let mut det = text_detector();
    for _ in 0..3 {
        let _ = det.record("looped");
    }
    assert!(det.detect().is_some(), "trips on the repeated text");
    det.reset(); // the recovery path resets the detector
    assert!(
        det.record("a completely different response").is_none(),
        "a different response after recovery must not re-trip on the stale key"
    );
    // But a genuine re-loop still trips after the threshold.
    let _ = det.record("looped");
    let _ = det.record("looped");
    let _ = det.record("looped");
    assert!(det.detect().is_some(), "a true re-loop must still trip");
}

#[test]
fn text_detector_zero_window_and_threshold_clamped() {
    let mut det = TextLoopDetector::new(0, 0);
    assert!(det.record("x").is_some(), "threshold 0 → 1: a single response trips");
}

#[test]
fn text_detect_reports_active_loop_key() {
    // Two keys both reach the threshold; the one with the newest occurrence
    // (the loop the model is in now) is reported.
    let mut det = TextLoopDetector::new(10, 2);
    for _ in 0..2 {
        let _ = det.record("old-loop");
    }
    for _ in 0..2 {
        let _ = det.record("new-loop");
    }
    let report = det.detect().expect("must trip");
    assert_eq!(report.canonical_args, "new-loop");
}

// ---------------------------------------------------------------------------
// LoopReport::summary + text_loop_recovery_prompt
// ---------------------------------------------------------------------------

#[test]
fn summary_tool_call_unchanged() {
    let report = crate::agent::LoopReport {
        kind: LoopKind::ToolCall,
        tool_name: "bash".to_string(),
        canonical_args: "{}".to_string(),
        args_preview: "grep -n foo".to_string(),
        count: 8,
    };
    assert_eq!(report.summary(), "bash \"grep -n foo\"");
}

#[test]
fn summary_text_echo_names_response() {
    let report = crate::agent::LoopReport {
        kind: LoopKind::TextEcho,
        tool_name: "response".to_string(),
        canonical_args: "I need to investigate".to_string(),
        args_preview: "I need to investigate".to_string(),
        count: 3,
    };
    assert_eq!(report.summary(), "repeated response \"I need to investigate\"");
}

#[test]
fn text_recovery_prompt_names_text_and_budget() {
    let prompt = text_loop_recovery_prompt("I need to investigate…", 603, 1, 3);
    assert!(prompt.contains("I need to investigate…"), "must name the repeated text");
    assert!(prompt.contains("603"), "must state the count");
    assert!(prompt.contains("1 of 3"), "must show the recovery budget");
    // It must steer toward a different action, not a repeat.
    assert!(prompt.to_lowercase().contains("different"));
}

#[test]
fn text_recovery_prompt_is_not_empty() {
    let prompt = text_loop_recovery_prompt("done", 3, 2, 3);
    assert_ne!(prompt.trim(), "");
    assert!(prompt.contains("2 of 3"));
}

// ---------------------------------------------------------------------------
// Regression: session 5b6b68d3 pattern
// ---------------------------------------------------------------------------

/// Regression test for the real text-echo loop in session `5b6b68d3`:
/// the same text-only response ("I need to investigate this further. Let me
/// check the details. …") repeated 602× with zero tool calls, driven by a
/// Stop-hook re-prompt cycle. A consecutive-run detector would trip, but the
/// tool-call detector never sees it (no calls). The text detector must trip
/// at the 3rd identical response.
#[test]
fn regression_5b6b68d3_text_echo_trips_at_third() {
    let mut det = text_detector();
    const RESPONSE: &str = "I need to investigate this further. Let me check the details.\
        I can see the `update_snapshot` system.";
    let mut tripped_at: Option<usize> = None;

    for i in 1..=10 {
        // The real response spanned multiple lines; normalization must keep
        // it a single stable key.
        let _ = det.record(&format!("{RESPONSE}\n"));
        if tripped_at.is_none() && det.detect().is_some() {
            tripped_at = Some(i);
        }
    }

    let tripped_at = tripped_at.expect("the text-echo loop must trip the detector");
    assert_eq!(tripped_at, 3, "must trip at exactly the 3rd identical response");
    let report = det.detect().expect("detector must be tripped");
    assert_eq!(report.kind, LoopKind::TextEcho);
    assert!(report.canonical_args.contains("update_snapshot"));
    assert_eq!(report.count, 10);
}
