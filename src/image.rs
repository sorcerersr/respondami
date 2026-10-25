//! Image normalization pipeline for vision support.
//!
//! Converts an image file into a base64 [`ImageContent`] block that the LLM
//! layer can serialize. All the messy work (MIME sniff, resize, re-encode,
//! byte cap) happens here at the ingestion boundary; the provider stays a pure
//! serializer.
//!
//! The base64 is frozen at this point, so the stored value is byte-stable for
//! the life of the message (vLLM prefix-cache stability). Encoder
//! non-determinism can never matter because the result is stored, not
//! recomputed.

use std::io::Cursor;
use std::path::Path;

use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use image::ImageFormat;
use thiserror::Error;

/// A normalized image payload: base64 data + MIME type + post-resize
/// dimensions.
///
/// This is the only image type the LLM/provider layer ever sees. The
/// dimensions are stored so token estimation works on resume without
/// re-decoding the base64.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq)]
pub struct ImageContent {
    /// Base64-encoded image bytes.
    pub data: String,
    /// MIME type, e.g. `"image/png"`.
    pub mime_type: String,
    /// Post-resize width in pixels.
    pub width: u32,
    /// Post-resize height in pixels.
    pub height: u32,
}

/// Errors from the image normalization pipeline.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum ImageError {
    /// The file's magic bytes don't match a supported format
    /// (JPEG/PNG/static-GIF/WebP).
    #[error("unsupported image format")]
    Unsupported,
    /// The encoded payload exceeds the byte cap.
    #[error("image too large after encoding")]
    TooLarge,
    /// I/O failure reading the file.
    #[error("io error: {0}")]
    Io(String),
    /// The image could not be decoded.
    #[error("failed to decode image")]
    DecodeFailed,
}

/// Maximum post-resize dimension (width or height), preserving aspect ratio.
pub const MAX_DIMENSION: u32 = 2000;

/// Maximum encoded payload size in bytes (4.5 MB, below the strictest provider
/// limit).
pub const MAX_ENCODED_BYTES: usize = 4_500_000;

/// Sniff the MIME type from magic bytes. Never trusts the file extension.
pub fn sniff_mime(bytes: &[u8]) -> Result<&'static str, ImageError> {
    if bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
        Ok("image/jpeg")
    } else if bytes.starts_with(&[0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A]) {
        Ok("image/png")
    } else if bytes.starts_with(b"GIF8") {
        Ok("image/gif")
    } else if bytes.len() >= 12 && &bytes[0..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
        Ok("image/webp")
    } else {
        Err(ImageError::Unsupported)
    }
}

/// Normalize an image file into a base64 [`ImageContent`].
///
/// Pipeline: read → sniff → (reject animated GIF) → decode → resize (2000px,
/// Lanczos3) → re-encode (JPEG; PNG/GIF/WebP → PNG) → byte cap → base64.
pub fn process_image(path: &Path) -> Result<ImageContent, ImageError> {
    let bytes = std::fs::read(path).map_err(|e| ImageError::Io(e.to_string()))?;
    let mime = sniff_mime(&bytes)?;
    if mime == "image/gif" && is_animated_gif(&bytes)? {
        return Err(ImageError::Unsupported);
    }
    let img = image::load_from_memory(&bytes).map_err(|e| ImageError::Io(format!("decode failed: {e}")))?;
    let resized = if img.width() > MAX_DIMENSION || img.height() > MAX_DIMENSION {
        img.resize(
            MAX_DIMENSION,
            MAX_DIMENSION,
            image::imageops::FilterType::Lanczos3,
        )
    } else {
        img
    };
    let (width, height) = (resized.width(), resized.height());
    let encoded = re_encode(&resized, mime)?;
    if encoded.len() > MAX_ENCODED_BYTES {
        return Err(ImageError::TooLarge);
    }
    Ok(ImageContent {
        data: STANDARD.encode(&encoded),
        mime_type: mime.to_string(),
        width,
        height,
    })
}

/// Estimate the token cost of an image from its post-resize dimensions.
///
/// Heuristic: `1600 + (w·h / 1024²) × 448`. The provider's real
/// `prompt_tokens` corrects the running total.
#[must_use]
pub fn estimate_image_tokens(width: u32, height: u32) -> u32 {
    let area = (width as u64) * (height as u64);
    1600 + ((area / (1024 * 1024)) as u32) * 448
}

/// Re-encode the resized image. JPEG stays JPEG; PNG/GIF/WebP are re-encoded
/// to PNG (lossless, universally accepted). Default encoder quality is used —
/// the byte cap bounds size and the base64 is frozen at this point.
fn re_encode(img: &image::DynamicImage, mime: &str) -> Result<Vec<u8>, ImageError> {
    let format = if mime == "image/jpeg" {
        ImageFormat::Jpeg
    } else {
        ImageFormat::Png
    };
    let mut buf = Cursor::new(Vec::new());
    img.write_to(&mut buf, format).map_err(|e| ImageError::Io(format!("encode failed: {e}")))?;
    Ok(buf.into_inner())
}

/// Returns `true` if the GIF has more than one frame (animated).
///
/// Walks the GIF structure instead of counting `0x2C` bytes (which appear in
/// LZW image data and cause false positives on valid static GIFs): header →
/// logical screen descriptor (+ global color table) → a loop over blocks. Each
/// image descriptor (`0x2C`) is one frame; its local color table and LZW
/// sub-blocks are skipped. Extension blocks (`0x21` + label) and the trailer
/// (`0x3B`) are skipped. A malformed structure returns `Ok(false)` — the
/// `image` crate decoder remains the authority (it returns `ImageError::Io`
/// on garbage); the walk only gates the animated-reject.
pub(crate) fn is_animated_gif(bytes: &[u8]) -> Result<bool, ImageError> {
    // Header "GIF87a"/"GIF89a" (6 bytes) + logical screen descriptor (7).
    if bytes.len() < 13 {
        return Ok(false);
    }
    let mut i = 13usize;

    // Global color table, if the packed byte's high bit is set.
    let packed = bytes[10];
    if packed & 0x80 != 0 {
        let gct = 3 * (1usize << ((packed & 0x07) + 1));
        match i.checked_add(gct) {
            Some(n) if n <= bytes.len() => i = n,
            _ => return Ok(false),
        }
    }

    let mut frames = 0;
    while i < bytes.len() {
        match bytes[i] {
            0x3B => return Ok(frames > 1),
            0x2C => {
                // Image descriptor: marker + left(2) top(2) width(2) height(2) packed(1).
                if i + 10 > bytes.len() {
                    return Ok(false);
                }
                frames += 1;
                if frames > 1 {
                    return Ok(true);
                }
                let lct_packed = bytes[i + 9];
                i += 10;
                if lct_packed & 0x80 != 0 {
                    let lct = 3 * (1usize << ((lct_packed & 0x07) + 1));
                    match i.checked_add(lct) {
                        Some(n) if n <= bytes.len() => i = n,
                        _ => return Ok(false),
                    }
                }
                // LZW minimum code size (1 byte), then the image data sub-blocks.
                if i >= bytes.len() {
                    return Ok(false);
                }
                i += 1;
                match skip_sub_blocks(bytes, i) {
                    Some(n) => i = n,
                    None => return Ok(false),
                }
            }
            0x21 => {
                // Extension: marker + label, then fixed data, then sub-blocks.
                if i + 2 > bytes.len() {
                    return Ok(false);
                }
                let fixed = match bytes[i + 1] {
                    0xF9 => 5,  // graphic control: block size + 4 data bytes
                    0xFF => 12, // application: block size + 11 data bytes
                    _ => 0,     // comment (0xFE) or plain extension
                };
                match i.checked_add(2 + fixed) {
                    Some(n) if n <= bytes.len() => i = n,
                    _ => return Ok(false),
                }
                match skip_sub_blocks(bytes, i) {
                    Some(n) => i = n,
                    None => return Ok(false),
                }
            }
            _ => return Ok(false),
        }
    }
    Ok(frames > 1)
}

/// Skip a run of GIF sub-blocks: a sequence of (1-byte length, `length` bytes
/// of data), terminated by a zero-length block. Returns the index past the
/// terminator, or `None` if the run is truncated.
fn skip_sub_blocks(bytes: &[u8], mut i: usize) -> Option<usize> {
    loop {
        let len = *bytes.get(i)? as usize;
        i = i.checked_add(1)?;
        if len == 0 {
            return Some(i);
        }
        i = i.checked_add(len)?;
        if i > bytes.len() {
            return None;
        }
    }
}
