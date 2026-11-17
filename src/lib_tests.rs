//! Tests for the dropped-image-path detection and the image-paste key intercept
//! in the main event loop.

use std::path::PathBuf;

use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyEventState, KeyModifiers};

use crate::config::Config;
use crate::session::SessionStore;
use crate::tui::App;

fn make_key(code: KeyCode, modifiers: KeyModifiers) -> KeyEvent {
    KeyEvent {
        code,
        modifiers,
        kind: KeyEventKind::Press,
        state: KeyEventState::NONE,
    }
}

fn make_app() -> App {
    // Re-point the session store at a temp dir so no real session files are
    // written (the drop-attach path never creates a session, but keep the
    // convention for consistency with the other App tests).
    let dir = tempfile::tempdir().unwrap().keep();
    let mut app = App::new(Config::default(), PathBuf::from("."));
    app.session.session_store = SessionStore::new(&dir);
    app
}

/// A minimal valid PNG (1x1) written via the `image` crate.
fn write_png(dir: &std::path::Path, name: &str) -> std::path::PathBuf {
    let path = dir.join(name);
    let img = image::RgbaImage::from_pixel(1, 1, image::Rgba([0, 0, 0, 255]));
    img.save(&path).unwrap();
    path
}

#[test]
fn dropped_single_image_path_attaches() {
    let dir = tempfile::tempdir().unwrap();
    let path = write_png(dir.path(), "shot.png");
    let mut app = make_app();

    let attached = crate::try_attach_dropped_image(&mut app, &path.to_string_lossy());

    assert!(attached, "single image path must attach");
    assert_eq!(app.editor.pending_images.len(), 1);
    assert_eq!(app.editor.pending_images[0].name, "shot.png");
    // A token was inserted at the cursor (ids start at 0).
    assert_eq!(app.editor.input_buffer, "[img:0]");
    assert_eq!(app.editor.cursor_pos, 7);
}

#[test]
fn dropped_path_plus_text_is_plain_insert() {
    let dir = tempfile::tempdir().unwrap();
    let path = write_png(dir.path(), "shot.png");
    let mut app = make_app();

    let text = format!("{} and some text", path.display());
    let attached = crate::try_attach_dropped_image(&mut app, &text);

    assert!(!attached, "path + text must not attach");
    assert!(app.editor.pending_images.is_empty());
    assert!(app.editor.input_buffer.is_empty(), "no token inserted");
}

#[test]
fn dropped_multi_path_is_plain_insert() {
    let dir = tempfile::tempdir().unwrap();
    let a = write_png(dir.path(), "a.png");
    let b = write_png(dir.path(), "b.png");
    let mut app = make_app();

    // Two paths separated by whitespace → not a single path.
    let text = format!("{} {}", a.display(), b.display());
    let attached = crate::try_attach_dropped_image(&mut app, &text);

    assert!(!attached, "two paths must not attach");
    assert!(app.editor.pending_images.is_empty());
}

#[test]
fn dropped_missing_path_is_plain_insert() {
    let mut app = make_app();
    let attached =
        crate::try_attach_dropped_image(&mut app, "/nonexistent/path/shot.png");

    assert!(!attached, "missing path must not attach");
    assert!(app.editor.pending_images.is_empty());
}

#[test]
fn dropped_non_image_file_is_plain_insert() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("notes.txt");
    std::fs::write(&path, "just text").unwrap();
    let mut app = make_app();

    let attached = crate::try_attach_dropped_image(&mut app, &path.to_string_lossy());

    assert!(!attached, "non-image file must not attach");
    assert!(app.editor.pending_images.is_empty());
}

#[test]
fn dropped_image_with_vision_off_is_plain_insert() {
    let dir = tempfile::tempdir().unwrap();
    let path = write_png(dir.path(), "shot.png");
    let mut app = make_app();
    app.config.config.provider.models[0].vision = false;

    let attached = crate::try_attach_dropped_image(&mut app, &path.to_string_lossy());

    assert!(!attached, "vision off must not attach");
    assert!(app.editor.pending_images.is_empty());
}

#[test]
fn image_paste_key_matches_ctrl_v() {
    let key = make_key(KeyCode::Char('v'), KeyModifiers::CONTROL);
    assert!(
        crate::is_image_paste_key(&key),
        "Ctrl+V must be the image-paste key"
    );
}

#[test]
fn image_paste_key_matches_ctrl_shift_v() {
    let key = make_key(
        KeyCode::Char('v'),
        KeyModifiers::CONTROL | KeyModifiers::SHIFT,
    );
    assert!(
        crate::is_image_paste_key(&key),
        "Ctrl+Shift+V must still be the image-paste key"
    );
}

#[test]
fn image_paste_key_rejects_plain_v() {
    let key = make_key(KeyCode::Char('v'), KeyModifiers::NONE);
    assert!(
        !crate::is_image_paste_key(&key),
        "plain V must not trigger image paste"
    );
}

#[test]
fn image_paste_key_rejects_shift_v() {
    let key = make_key(KeyCode::Char('v'), KeyModifiers::SHIFT);
    assert!(
        !crate::is_image_paste_key(&key),
        "Shift+V (no Ctrl) must not trigger image paste"
    );
}

#[test]
fn image_paste_key_rejects_ctrl_other_char() {
    let key = make_key(KeyCode::Char('c'), KeyModifiers::CONTROL);
    assert!(
        !crate::is_image_paste_key(&key),
        "Ctrl+C must not trigger image paste"
    );
}

/// A minimal valid PNG (1x1) as raw bytes, for the 5522 attach tests.
fn png_bytes() -> Vec<u8> {
    let img = image::RgbaImage::from_pixel(1, 1, image::Rgba([0, 0, 0, 255]));
    let mut buf = Vec::new();
    img.write_to(&mut std::io::Cursor::new(&mut buf), image::ImageFormat::Png)
        .unwrap();
    buf
}

#[test]
fn attach_5522_image_attaches_token() {
    let mut app = make_app();
    crate::attach_5522_image(&mut app, png_bytes());
    assert_eq!(app.editor.pending_images.len(), 1);
    assert_eq!(app.editor.input_buffer, "[img:0]");
    assert_eq!(app.editor.cursor_pos, 7);
}

#[test]
fn attach_5522_image_vision_off_no_attach() {
    let mut app = make_app();
    app.config.config.provider.models[0].vision = false;
    crate::attach_5522_image(&mut app, png_bytes());
    assert_eq!(app.editor.pending_images.len(), 0);
    assert_eq!(app.editor.input_buffer.len(), 0);
    // A system message is shown.
    assert!(!app.chat.chat_messages.is_empty());
}

#[test]
fn attach_5522_image_malformed_bytes_no_attach() {
    let mut app = make_app();
    crate::attach_5522_image(&mut app, b"not an image".to_vec());
    assert_eq!(app.editor.pending_images.len(), 0);
    assert_eq!(app.editor.input_buffer.len(), 0);
    // A system message is shown.
    assert!(!app.chat.chat_messages.is_empty());
}
