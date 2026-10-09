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
/// A GIF has exactly one image-descriptor byte (`0x2C`) per frame, and that
/// byte is not used by any other GIF block type (extension `0x21`, palette
/// `0x00`, trailer `0x3B`).
fn is_animated_gif(bytes: &[u8]) -> Result<bool, ImageError> {
    let mut frames = 0;
    for &b in bytes {
        if b == 0x2C {
            frames += 1;
            if frames > 1 {
                return Ok(true);
            }
        }
    }
    Ok(false)
}
