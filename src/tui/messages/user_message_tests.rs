// Tests for UserMessage height computation.

use super::user_message::UserMessage;
use super::Theme;
use ratatui_md::HeightAware;

fn make_theme() -> Theme {
    Theme::gh_dark()
}

#[test]
fn height_single_line_short() {
    let msg = UserMessage { content: "hello".to_string() };
    assert_eq!(msg.height(80, &make_theme()), 4); // 1 row + 3 spacing
}

#[test]
fn height_wraps_long_line() {
    let msg = UserMessage { content: "a".repeat(160) };
    assert_eq!(msg.height(80, &make_theme()), 5); // 2 rows + 3 spacing
}

#[test]
fn height_zero_width() {
    let msg = UserMessage { content: "hello".to_string() };
    assert_eq!(msg.height(0, &make_theme()), 4);
}

#[test]
fn m1_height_cjk_uses_display_width() {
    // 10 CJK chars = 20 display cells. At width 10 → 2 rows (a char count of 10
    // would give 1 row and clip the render). M-1. +3 spacing = 5.
    let msg = UserMessage { content: "日本語の文字は広いよ".to_string() };
    assert_eq!(msg.height(10, &make_theme()), 5);
    // Exactly fitting display width stays 1 row + 3 spacing = 4.
    assert_eq!(msg.height(20, &make_theme()), 4);
}
