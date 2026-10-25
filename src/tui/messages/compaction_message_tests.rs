// Tests for CompactionMessage height computation.

use super::compaction_message::CompactionMessage;
use super::Theme;
use ratatui_md::HeightAware;

fn make_theme() -> Theme {
    Theme::gh_dark()
}

#[test]
fn height_short_width_fits() {
    let msg = CompactionMessage { tokens_saved: 100, message_count: 5 };
    // "▸ Compacted 5 messages (saved 100 tokens)" = 43 display cells
    assert_eq!(msg.height(80, &make_theme()), 2); // 1 row + 1 blank
}

#[test]
fn height_wraps_when_narrow() {
    let msg = CompactionMessage { tokens_saved: 1000, message_count: 50 };
    // 43 display cells at width 20 → 3 rows + 1 blank = 4
    assert_eq!(msg.height(20, &make_theme()), 4);
}

#[test]
fn height_zero_width() {
    let msg = CompactionMessage { tokens_saved: 100, message_count: 5 };
    assert_eq!(msg.height(0, &make_theme()), 2);
}

#[test]
fn m1_height_uses_display_width() {
    // The compaction text is ASCII, but the height must still be measured by
    // display width (M-1): 43 cells at width 10 → 5 rows + 1 blank = 6.
    let msg = CompactionMessage { tokens_saved: 1000, message_count: 50 };
    assert_eq!(msg.height(10, &make_theme()), 6);
}
