//! Tests for the image-token handling module.

use super::tokens::{
    TokenSpan, find_tokens, format_token, strip_tokens, token_ending_at, token_starting_at,
};

#[test]
fn format_token_produces_expected_shape() {
    assert_eq!(format_token(0), "[img:0]");
    assert_eq!(format_token(42), "[img:42]");
}

#[test]
fn find_tokens_none_when_absent() {
    assert_eq!(find_tokens("hello world"), Vec::new());
    assert_eq!(find_tokens(""), Vec::new());
}

#[test]
fn find_tokens_single() {
    let tokens = find_tokens("before [img:3] after");
    assert_eq!(
        tokens,
        vec![TokenSpan {
            start: 7,
            end: 14,
            id: 3
        }]
    );
}

#[test]
fn find_tokens_multiple_in_order() {
    let tokens = find_tokens("[img:0] mid [img:1]");
    assert_eq!(
        tokens,
        vec![
            TokenSpan {
                start: 0,
                end: 7,
                id: 0
            },
            TokenSpan {
                start: 12,
                end: 19,
                id: 1
            }
        ]
    );
}

#[test]
fn find_tokens_ignores_malformed() {
    // No closing bracket, no digits, wrong prefix.
    assert_eq!(find_tokens("[img:]"), Vec::new());
    assert_eq!(find_tokens("[img:abc]"), Vec::new());
    assert_eq!(find_tokens("[im:0]"), Vec::new());
}

#[test]
fn token_starting_at_finds_at_start() {
    let input = "x [img:5] y";
    let token = token_starting_at(input, 2).unwrap();
    assert_eq!(token.id, 5);
    assert_eq!((token.start, token.end), (2, 9));
    // Not at a non-start position.
    assert!(token_starting_at(input, 3).is_none());
    assert!(token_starting_at(input, 0).is_none());
}

#[test]
fn token_ending_at_finds_at_end() {
    let input = "x [img:5] y";
    let token = token_ending_at(input, 9).unwrap();
    assert_eq!(token.id, 5);
    // Not at a non-end position.
    assert!(token_ending_at(input, 2).is_none());
    assert!(token_ending_at(input, 10).is_none());
}

#[test]
fn strip_tokens_removes_all() {
    assert_eq!(strip_tokens("a [img:0] b [img:1] c"), "a  b  c");
    assert_eq!(strip_tokens("no tokens"), "no tokens");
    assert_eq!(strip_tokens("[img:0]"), "");
}

#[test]
fn strip_tokens_preserves_non_token_text() {
    // Text that looks similar but is not a valid token is preserved.
    assert_eq!(strip_tokens("see [img: for info"), "see [img: for info");
}
