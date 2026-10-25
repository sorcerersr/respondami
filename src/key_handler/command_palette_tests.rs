// Extracted tests from src/key_handler/command_palette.rs
// Pins the L-1 fix: `update_scroll` runs after navigation, so the scroll
// offset advances when the selection moves past the first page.

use crate::config::Config;
use crate::key_handler::command_palette::update_scroll;
use crate::tui::App;

fn make_app() -> App {
    App::new(Config::default(), std::path::PathBuf::from("."))
}

#[test]
fn update_scroll_advances_past_first_page() {
    // Simulate a palette with more commands than fit on one page
    // (PALETTE_PAGE_SIZE = 15). Selecting item 16 from offset 0 must scroll.
    let mut app = make_app();
    app.modal.command_palette_selected = 16;
    app.modal.command_palette_scroll_offset = 0;
    update_scroll(&mut app, 20);
    // 16 - 15 + 1 = 2
    assert_eq!(app.modal.command_palette_scroll_offset, 2);
}

#[test]
fn update_scroll_clamps_to_max_offset() {
    // Selecting the last item must not scroll past the valid range.
    let mut app = make_app();
    app.modal.command_palette_selected = 19;
    app.modal.command_palette_scroll_offset = 0;
    update_scroll(&mut app, 20);
    // 19 - 15 + 1 = 5, clamped to total - page = 20 - 15 = 5
    assert_eq!(app.modal.command_palette_scroll_offset, 5);
}

#[test]
fn update_scroll_no_change_when_in_view() {
    // A selection already within the visible window leaves the offset alone.
    let mut app = make_app();
    app.modal.command_palette_selected = 3;
    app.modal.command_palette_scroll_offset = 0;
    update_scroll(&mut app, 20);
    assert_eq!(app.modal.command_palette_scroll_offset, 0);
}

#[test]
fn update_scroll_scrolls_up_when_selection_above_offset() {
    // Moving the selection above the current window top scrolls back up.
    let mut app = make_app();
    app.modal.command_palette_selected = 2;
    app.modal.command_palette_scroll_offset = 5;
    update_scroll(&mut app, 20);
    assert_eq!(app.modal.command_palette_scroll_offset, 2);
}
