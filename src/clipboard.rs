//! Clipboard image fetching for paste support.
//!
//! Wraps `arboard` to read an image from the platform clipboard (X11 via
//! `x11rb`, Wayland via `wayland-client` — no system libraries required) and
//! normalize it through the shared [`crate::image::process_image_bytes`]
//! pipeline, so pasted and read images are handled identically.
//!
//! `arboard`'s `get_image` returns the image as raw RGBA8 bytes (it reads the
//! clipboard's PNG and decodes it), so we re-encode to PNG in memory and pass
//! the result through `process_image_bytes` (sniff/resize/cap/base64). The
//! double-encode is intentional — it keeps a single normalization path shared
//! with file ingestion.
//!
//! The clipboard handle is cached for the process lifetime: connection setup
//! is the expensive part, and a failed creation (no display server, e.g.
//! headless SSH) is negative-cached so repeated presses don't retry the
//! connection and spam the error.
//!
//! All functions here are blocking (arboard is synchronous) and must be called
//! from `spawn_blocking` in the async event loop.

use std::sync::{Arc, LazyLock, Mutex};

use arboard::{Clipboard, ImageData};
use image::{ImageFormat, RgbaImage};
use thiserror::Error;

use crate::image::{ImageContent, process_image_bytes};

/// Errors from clipboard image fetching.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum ClipboardError {
    /// The clipboard has no image (text-only or empty).
    #[error("no image on clipboard")]
    NoImage,
    /// No display server is available (headless).
    #[error("clipboard unavailable (no display server)")]
    NoDisplay,
    /// The image could not be normalized (unsupported format, oversized, or a
    /// malformed payload from the clipboard).
    #[error("image normalization failed: {0}")]
    Normalization(String),
}

/// A cached, shareable clipboard handle. `Mutex` provides the `&mut` that
/// `get_image` requires; `Arc` lets the handle be cloned cheaply.
type Board = Arc<Mutex<Clipboard>>;

/// Cached clipboard handle.
///
/// `None` = never tried; `Some(Ok)` = ready to use; `Some(Err)` = negative
/// cache (creation failed, e.g. no display server) — do not retry.
static CLIPBOARD: LazyLock<Mutex<Option<Result<Board, ()>>>> =
    LazyLock::new(|| Mutex::new(None));

/// Get the cached clipboard handle, creating it on first use.
///
/// A creation failure is negative-cached so subsequent calls return
/// [`ClipboardError::NoDisplay`] without retrying the connection.
fn get_board() -> Result<Board, ClipboardError> {
    let mut guard = CLIPBOARD.lock().map_err(|_e| ClipboardError::NoDisplay)?;
    match guard.as_ref() {
        Some(Ok(board)) => Ok(board.clone()),
        Some(Err(())) => Err(ClipboardError::NoDisplay),
        None => match Clipboard::new() {
            Ok(cb) => {
                let board = Arc::new(Mutex::new(cb));
                *guard = Some(Ok(board.clone()));
                Ok(board)
            }
            Err(e) => {
                tracing::debug!("clipboard init failed (negative-cached): {e}");
                *guard = Some(Err(()));
                Err(ClipboardError::NoDisplay)
            }
        },
    }
}

/// Fetch and normalize an image from the platform clipboard.
///
/// Returns `Ok(Some(image))` when the clipboard holds an image, `Ok(None)`
/// when it holds no image (text-only or empty), and `Err` for a missing
/// display server or a payload that fails normalization.
///
/// # Errors
///
/// Returns [`ClipboardError`] as described by its variants.
pub fn fetch_image() -> Result<Option<ImageContent>, ClipboardError> {
    let board = get_board()?;
    let img: ImageData = {
        let mut cb = board.lock().map_err(|_e| ClipboardError::NoDisplay)?;
        match cb.get_image() {
            Ok(img) => img,
            Err(e) => {
                // A missing image is the common case; a real I/O failure is
                // rare. Both surface as "no image" for the caller (a
                // convenience feature should not produce a red error card).
                tracing::debug!("clipboard get_image: {e}");
                return Ok(None);
            }
        }
    };
    Ok(Some(rgba_to_content(img)?))
}

/// Convert a raw RGBA8 clipboard payload into a normalized [`ImageContent`].
///
/// `arboard` delivers the clipboard image as raw RGBA8 bytes; we wrap them in
/// an [`RgbaImage`], encode to PNG in memory, and pass the result through
/// [`process_image_bytes`] to reuse the full sniff/resize/cap/base64 pipeline.
pub(crate) fn rgba_to_content(img: ImageData) -> Result<ImageContent, ClipboardError> {
    let bytes = img.bytes.into_owned();
    let rgba = RgbaImage::from_raw(img.width as u32, img.height as u32, bytes)
        .ok_or_else(|| ClipboardError::Normalization("malformed RGBA payload".to_string()))?;
    let mut buf = std::io::Cursor::new(Vec::new());
    rgba.write_to(&mut buf, ImageFormat::Png)
        .map_err(|e| ClipboardError::Normalization(format!("png encode failed: {e}")))?;
    process_image_bytes(&buf.into_inner())
        .map_err(|e| ClipboardError::Normalization(e.to_string()))
}

/// The terminal emulator, detected from environment variables at startup.
///
/// 5522 is enabled only for Kitty and Ghostty; all other terminals fall back
/// to the existing arboard `Ctrl+V` path.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TerminalKind {
    /// Kitty terminal (`TERM_PROGRAM=kitty`).
    Kitty,
    /// Ghostty terminal (`TERM=xterm-ghostty`).
    Ghostty,
    /// Any other terminal (or detection failure) — the default.
    #[default]
    Other,
}

/// Detect the terminal emulator from environment variables.
///
/// Checks `TERM_PROGRAM` (Kitty) and `TERM` (Ghostty) in that order. Returns
/// [`TerminalKind::Other`] when neither matches, so the existing arboard path
/// is used as a fallback.
#[must_use]
pub fn detect_terminal() -> TerminalKind {
    let term_program = std::env::var("TERM_PROGRAM").ok();
    let term = std::env::var("TERM").ok();
    detect_terminal_from(term_program.as_deref(), term.as_deref())
}

/// Pure form of [`detect_terminal`] that takes the env values directly, so it
/// can be tested without modifying the process environment.
#[must_use]
pub(crate) fn detect_terminal_from(term_program: Option<&str>, term: Option<&str>) -> TerminalKind {
    if term_program == Some("kitty") {
        TerminalKind::Kitty
    } else if term == Some("xterm-ghostty") {
        TerminalKind::Ghostty
    } else {
        TerminalKind::Other
    }
}
