use super::edit_diff::*;

// ===== BOM Tests =====

#[test]
fn strip_bom_removes_bom() {
    let (had_bom, text) = strip_bom("\u{FEFF}hello");
    assert!(had_bom);
    assert_eq!(text, "hello");
}

#[test]
fn strip_bom_no_bom() {
    let (had_bom, text) = strip_bom("hello");
    assert!(!had_bom);
    assert_eq!(text, "hello");
}

#[test]
fn strip_bom_empty() {
    let (had_bom, text) = strip_bom("");
    assert!(!had_bom);
    assert_eq!(text, "");
}

#[test]
fn bom_round_trip() {
    let original = "\u{FEFF}hello world";
    let (had_bom, text) = strip_bom(original);
    assert!(had_bom);
    let restored = if had_bom { format!("\u{FEFF}{text}") } else { text.to_string() };
    assert_eq!(restored, original);
}

// ===== Line Ending Tests =====

#[test]
fn detect_line_ending_crlf() {
    assert_eq!(detect_line_ending("a\r\nb\r\n"), "\r\n");
}

#[test]
fn detect_line_ending_lf() {
    assert_eq!(detect_line_ending("a\nb\n"), "\n");
}

#[test]
fn detect_line_ending_mixed_dominant_crlf() {
    // Early bare LF, but CRLF is dominant → CRLF wins
    assert_eq!(detect_line_ending("a\nb\r\nc\r\nd\r\n"), "\r\n");
}

#[test]
fn detect_line_ending_mixed_dominant_lf() {
    // Early CRLF, but bare LF is dominant → LF wins
    assert_eq!(detect_line_ending("a\r\nb\nc\nd\n"), "\n");
}

#[test]
fn detect_line_ending_tie_goes_to_crlf() {
    // Equal CRLF and bare LF → CRLF wins (safer for Windows files)
    assert_eq!(detect_line_ending("a\r\nb\nc"), "\r\n");
}

#[test]
fn detect_line_ending_no_newlines() {
    assert_eq!(detect_line_ending("hello"), "\n");
}

#[test]
fn normalize_to_lf_converts_crlf() {
    assert_eq!(normalize_to_lf("a\r\nb\r\n"), "a\nb\n");
}

#[test]
fn normalize_to_lf_converts_cr() {
    assert_eq!(normalize_to_lf("a\rb"), "a\nb");
}

#[test]
fn restore_line_endings_crlf() {
    assert_eq!(restore_line_endings("a\nb\n", "\r\n"), "a\r\nb\r\n");
}

#[test]
fn restore_line_endings_lf_unchanged() {
    assert_eq!(restore_line_endings("a\nb\n", "\n"), "a\nb\n");
}

#[test]
fn line_ending_round_trip() {
    let original = "a\r\nb\r\nc\r\n";
    let ending = detect_line_ending(original);
    let normalized = normalize_to_lf(original);
    let restored = restore_line_endings(&normalized, ending);
    assert_eq!(restored, original);
}

// ===== Fuzzy Matching Tests =====

#[test]
fn normalize_for_fuzzy_strips_trailing_whitespace() {
    let result = normalize_for_fuzzy("hello  \nworld  \n");
    assert_eq!(result, "hello\nworld\n");
}

#[test]
fn normalize_for_fuzzy_smart_quotes() {
    let result = normalize_for_fuzzy("\u{201C}hello\u{201D}");
    assert_eq!(result, "\"hello\"");
}

#[test]
fn normalize_for_fuzzy_smart_single_quotes() {
    let result = normalize_for_fuzzy("\u{2018}hello\u{2019}");
    assert_eq!(result, "'hello'");
}

#[test]
fn normalize_for_fuzzy_em_dash() {
    let result = normalize_for_fuzzy("a\u{2014}b");
    assert_eq!(result, "a-b");
}

#[test]
fn normalize_for_fuzzy_en_dash() {
    let result = normalize_for_fuzzy("a\u{2013}b");
    assert_eq!(result, "a-b");
}

#[test]
fn normalize_for_fuzzy_nbsp() {
    let result = normalize_for_fuzzy("a\u{00A0}b");
    assert_eq!(result, "a b");
}

#[test]
fn fuzzy_find_text_exact_match() {
    let result = fuzzy_find_text("hello world", "world");
    assert!(result.is_some());
    let r = result.unwrap();
    assert!(!r.used_fuzzy);
    assert_eq!(r.index, 6);
    assert_eq!(r.match_length, 5);
}

#[test]
fn fuzzy_find_text_fuzzy_trailing_whitespace() {
    let content = "hello  \nworld";
    let old_text = "hello\nworld"; // no trailing spaces
    let result = fuzzy_find_text(content, old_text);
    assert!(result.is_some());
    let r = result.unwrap();
    assert!(r.used_fuzzy);
}

#[test]
fn fuzzy_find_text_fuzzy_smart_quotes() {
    let content = "\u{201C}hello\u{201D}";
    let old_text = "\"hello\"";
    let result = fuzzy_find_text(content, old_text);
    assert!(result.is_some());
    let r = result.unwrap();
    assert!(r.used_fuzzy);
}

#[test]
fn fuzzy_find_text_not_found() {
    let result = fuzzy_find_text("hello world", "xyz");
    assert!(result.is_none());
}

#[test]
fn fuzzy_find_text_prefer_exact_over_fuzzy() {
    let content = "hello  \nworld";
    let old_text = "hello  \nworld"; // exact match with trailing spaces
    let result = fuzzy_find_text(content, old_text);
    assert!(result.is_some());
    let r = result.unwrap();
    assert!(!r.used_fuzzy); // exact match preferred
}

#[test]
fn fuzzy_count_occurrences_exact() {
    assert_eq!(fuzzy_count_occurrences("aaa", "a"), 3);
    assert_eq!(fuzzy_count_occurrences("hello world hello", "hello"), 2);
    assert_eq!(fuzzy_count_occurrences("hello", "xyz"), 0);
}

#[test]
fn fuzzy_count_occurrences_with_normalization() {
    // Smart quotes normalized to ASCII
    let content = "\u{201C}a\u{201D} \u{201C}b\u{201D}";
    assert_eq!(fuzzy_count_occurrences(content, "\"a\""), 1);
}

// ===== apply_edits Tests =====

#[test]
fn apply_edits_exact_match() {
    let content = "hello world";
    let edits = vec![("world".to_string(), "rust".to_string())];
    let result = apply_edits(content, &edits, "test.txt");
    assert!(result.is_ok());
    let r = result.unwrap();
    assert_eq!(r.new_content, "hello rust");
}

#[test]
fn apply_edits_fuzzy_fallback() {
    let content = "hello  \nworld";
    let edits = vec![("hello\nworld".to_string(), "hello\nrust".to_string())];
    let result = apply_edits(content, &edits, "test.txt");
    result.unwrap();
}

#[test]
fn apply_edits_empty_old_text() {
    let content = "hello";
    let edits = vec![(String::new(), "world".to_string())];
    let result = apply_edits(content, &edits, "test.txt");
    assert!(result.is_err());
    assert!(matches!(result.unwrap_err(), EditError::EmptyOldText { .. }));
}

#[test]
fn apply_edits_not_found() {
    let content = "hello world";
    let edits = vec![("notfound".to_string(), "replaced".to_string())];
    let result = apply_edits(content, &edits, "test.txt");
    assert!(result.is_err());
    assert!(matches!(result.unwrap_err(), EditError::NotFound { .. }));
}

#[test]
fn apply_edits_not_unique() {
    let content = "hello hello";
    let edits = vec![("hello".to_string(), "world".to_string())];
    let result = apply_edits(content, &edits, "test.txt");
    assert!(result.is_err());
    assert!(matches!(result.unwrap_err(), EditError::NotUnique { .. }));
}

#[test]
fn apply_edits_no_change() {
    let content = "hello world";
    let edits = vec![("hello world".to_string(), "hello world".to_string())];
    let result = apply_edits(content, &edits, "test.txt");
    assert!(result.is_err());
    assert!(matches!(result.unwrap_err(), EditError::NoChange { .. }));
}

#[test]
fn apply_edits_multiple_edits() {
    let content = "foo bar baz";
    let edits = vec![
        ("foo".to_string(), "FOO".to_string()),
        ("baz".to_string(), "BAZ".to_string()),
    ];
    let result = apply_edits(content, &edits, "test.txt");
    assert!(result.is_ok());
    let r = result.unwrap();
    assert_eq!(r.new_content, "FOO bar BAZ");
}

#[test]
fn apply_edits_overlapping() {
    let content = "hello world foo bar";
    let edits = vec![
        ("hello wor".to_string(), "replaced".to_string()),
        ("rld foo".to_string(), "also_replaced".to_string()),
    ];
    let result = apply_edits(content, &edits, "test.txt");
    assert!(result.is_err());
    assert!(matches!(result.unwrap_err(), EditError::Overlapping { .. }));
}

// ===== Diff Tests =====

#[test]
fn generate_diff_produces_output() {
    let old = "line1\nline2\nline3\nline4\nline5\nline6\nline7\n";
    let new = "line1\nline2\nline3\nmodified\nline5\nline6\nline7\n";
    let diff = generate_diff(old, new, "test.txt");
    assert!(diff.contains("test.txt"), "diff should contain filename: {diff:?}");
    assert!(
        diff.contains("modified") || diff.contains("line4"),
        "diff should contain changed lines: {diff:?}"
    );
}

#[test]
fn generate_diff_no_changes_shows_summary() {
    let content = "same content";
    let diff = generate_diff(content, content, "test.txt");
    assert!(diff.contains("Edits applied to"));
}

// ===== Internal Mismatch Tests =====

#[test]
fn line_count_mismatch_returns_internal_mismatch() {
    let original = "line1\nline2\nline3\n";
    let base = "line1\nline2\n";
    let replacements: &[(usize, usize, &str)] = &[];

    let result = apply_replacements_preserving_unchanged_lines(
        original,
        base,
        replacements,
        "test.txt",
    );

    assert!(result.is_err());
    let err = result.unwrap_err();
    assert!(
        matches!(err, EditError::InternalMismatch { .. }),
        "Expected InternalMismatch, got: {err}"
    );

    if let EditError::InternalMismatch {
        path,
        original_lines,
        normalized_lines,
    } = err
    {
        assert_eq!(path, "test.txt");
        assert_eq!(original_lines, 3);
        assert_eq!(normalized_lines, 2);
    }
}

#[test]
fn internal_mismatch_error_message_is_descriptive() {
    let original = "a\nb\nc\n";
    let base = "x\ny\n";
    let replacements: &[(usize, usize, &str)] = &[];

    let result =
        apply_replacements_preserving_unchanged_lines(original, base, replacements, "file.rs");
    let err = result.unwrap_err();
    let msg = err.to_string();
    assert!(msg.contains("file.rs"), "Message should contain file path");
    assert!(msg.contains('3'), "Message should contain original line count");
    assert!(msg.contains('2'), "Message should contain normalized line count");
}
