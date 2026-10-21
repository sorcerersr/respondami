//! Tests for `CancelGuard` and `ToolRegistry` in `src/tools/mod.rs`.

use super::{CancelGuard, ToolOutput};

#[test]
fn tool_output_text_has_no_images() {
    let out = ToolOutput::text("hello");
    assert_eq!(out.content, "hello");
    assert_eq!(out.images, Vec::new());
}

#[test]
fn tool_output_default_is_empty() {
    let out = ToolOutput::default();
    assert_eq!(out.content, String::new());
    assert_eq!(out.images, Vec::new());
}

#[test]
fn tool_output_serializes_omitting_empty_images() {
    let out = ToolOutput::text("result");
    let json = serde_json::to_string(&out).unwrap();
    assert!(!json.contains("\"images\""), "empty images must be omitted");
    let decoded: ToolOutput = serde_json::from_str(&json).unwrap();
    assert_eq!(decoded, out);
}

#[test]
fn tool_output_with_images_roundtrip() {
    use crate::image::ImageContent;
    let img = ImageContent {
        data: "aW1hZ2U=".to_string(),
        mime_type: "image/png".to_string(),
        width: 100,
        height: 50,
    };
    let out = ToolOutput {
        content: "Image: x".to_string(),
        images: vec![img.clone()],
    };
    let json = serde_json::to_string(&out).unwrap();
    assert!(json.contains("\"images\""));
    let decoded: ToolOutput = serde_json::from_str(&json).unwrap();
    assert_eq!(decoded, out);
    assert_eq!(decoded.images[0], img);
}

#[test]
fn cancel_guard_new_not_cancelled() {
    let guard = CancelGuard::new();
    assert!(!guard.is_cancelled());
}

#[test]
fn cancel_guard_cancel_sets_flag() {
    let guard = CancelGuard::new();
    guard.cancel();
    assert!(guard.is_cancelled());
}

#[test]
fn cancel_guard_clone_shares_state() {
    let guard = CancelGuard::new();
    let clone = guard.clone();
    assert!(!clone.is_cancelled());
    guard.cancel();
    assert!(clone.is_cancelled(), "clone must see the cancellation");
}

#[test]
fn cancel_guard_clone_cancel_propagates() {
    let guard = CancelGuard::new();
    let clone = guard.clone();
    clone.cancel();
    assert!(guard.is_cancelled(), "original must see the cancellation");
}

#[test]
fn cancel_guard_cancel_idempotent() {
    let guard = CancelGuard::new();
    guard.cancel();
    guard.cancel();
    assert!(guard.is_cancelled());
}

#[test]
fn cancel_guard_default_not_cancelled() {
    let guard = CancelGuard::default();
    assert!(!guard.is_cancelled());
}
