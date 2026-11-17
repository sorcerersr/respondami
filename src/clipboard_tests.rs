//! Tests for the clipboard image module.
//!
//! The real clipboard is environment-dependent (X11/Wayland display server),
//! so these tests focus on the pure `rgba_to_content` normalization path and a
//! smoke test for `fetch_image` (asserts it returns a `Result` without
//! panicking, whatever the environment).

use std::borrow::Cow;

use super::clipboard::{
    ClipboardError, TerminalKind, detect_terminal_from, fetch_image, rgba_to_content,
};
use arboard::ImageData;
use base64::Engine;

/// Build a solid-color RGBA8 payload (the shape `arboard` delivers).
fn solid_rgba(w: u32, h: u32, rgba: [u8; 4]) -> ImageData<'static> {
    let mut bytes = Vec::with_capacity((w * h * 4) as usize);
    for _ in 0..(w * h) {
        bytes.extend_from_slice(&rgba);
    }
    ImageData {
        width: w as usize,
        height: h as usize,
        bytes: Cow::Owned(bytes),
    }
}

#[test]
fn rgba_small_preserves_dimensions() {
    let img = solid_rgba(10, 8, [255, 0, 0, 255]);
    let content = rgba_to_content(img).unwrap();
    assert_eq!((content.width, content.height), (10, 8));
    assert_eq!(content.mime_type, "image/png");
    // The base64 decodes to a valid PNG.
    let decoded = base64::engine::general_purpose::STANDARD.decode(&content.data).unwrap();
    assert!(decoded.starts_with(&[0x89, 0x50, 0x4E, 0x47]));
}

#[test]
fn rgba_large_is_resized() {
    let img = solid_rgba(4000, 2000, [0, 255, 0, 255]);
    let content = rgba_to_content(img).unwrap();
    // Longest side capped at 2000, aspect ratio preserved.
    assert_eq!(content.width, 2000);
    assert_eq!(content.height, 1000);
}

#[test]
fn rgba_malformed_payload_is_normalization_error() {
    // bytes length does not match width*height*4.
    let img = ImageData {
        width: 10,
        height: 10,
        bytes: Cow::Owned(vec![0, 0, 0, 255]),
    };
    assert!(matches!(
        rgba_to_content(img),
        Err(ClipboardError::Normalization(_))
    ));
}

#[test]
fn fetch_image_smoke_returns_result() {
    // Environment-dependent: headless → NoDisplay, image on clipboard →
    // Some, text-only → None. All are valid `Result`s; the assertion is that
    // it does not panic.
    let _ = fetch_image();
}

#[test]
fn detect_terminal_kitty() {
    assert_eq!(
        detect_terminal_from(Some("kitty"), Some("xterm-256color")),
        TerminalKind::Kitty
    );
}

#[test]
fn detect_terminal_ghostty() {
    assert_eq!(
        detect_terminal_from(None, Some("xterm-ghostty")),
        TerminalKind::Ghostty
    );
}

#[test]
fn detect_terminal_other() {
    assert_eq!(detect_terminal_from(None, Some("xterm-256color")), TerminalKind::Other);
    assert_eq!(detect_terminal_from(Some("alacritty"), None), TerminalKind::Other);
    assert_eq!(detect_terminal_from(None, None), TerminalKind::Other);
}

#[test]
fn detect_terminal_kitty_preferred_over_ghostty() {
    // TERM_PROGRAM wins over TERM when both are set.
    assert_eq!(
        detect_terminal_from(Some("kitty"), Some("xterm-ghostty")),
        TerminalKind::Kitty
    );
}
