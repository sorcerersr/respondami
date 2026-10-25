use super::image::{
    ImageError, estimate_image_tokens, is_animated_gif, process_image, sniff_mime,
};
use base64::Engine;
use image::RgbaImage;
use std::io::Cursor;
use std::path::Path;
use tempfile::TempDir;

/// Write a solid-color image to `path`; the format is inferred from the
/// extension.
fn write_image(path: &Path, w: u32, h: u32, rgb: [u8; 3]) {
    let img = image::RgbImage::from_pixel(w, h, image::Rgb([rgb[0], rgb[1], rgb[2]]));
    img.save(path).expect("failed to write test image");
}

/// Write a two-frame (animated) GIF to `path`.
fn write_animated_gif(path: &Path, w: u32, h: u32) {
    use image::codecs::gif::GifEncoder;
    let f1 = RgbaImage::from_pixel(w, h, image::Rgba([255, 0, 0, 255]));
    let f2 = RgbaImage::from_pixel(w, h, image::Rgba([0, 0, 255, 255]));
    let frame1 = image::Frame::from_parts(f1, 0, 0, image::Delay::from_numer_denom_ms(100, 1));
    let frame2 = image::Frame::from_parts(f2, 0, 0, image::Delay::from_numer_denom_ms(100, 1));
    let mut buf = Cursor::new(Vec::new());
    let mut encoder = GifEncoder::new(&mut buf);
    encoder.encode_frame(frame1).expect("frame 1");
    encoder.encode_frame(frame2).expect("frame 2");
    drop(encoder);
    std::fs::write(path, buf.into_inner()).expect("write animated gif");
}

/// Build a minimal static GIF whose LZW image data contains a `0x2C` byte.
/// This is the M-1 regression: the old byte-counting detector saw the `0x2C` in
/// the LZW stream and rejected a perfectly good static GIF. The structural walk
/// must count only image descriptors.
///
/// The LZW sub-block structure is valid (length + data + terminator) so the
/// walk skips it correctly; the LZW payload is arbitrary (the `image` decoder is
/// not involved in the `is_animated_gif` unit test).
fn static_gif_with_0x2c_lzw() -> Vec<u8> {
    let mut b: Vec<u8> = Vec::new();
    b.extend_from_slice(b"GIF89a");
    // Logical screen descriptor: 1x1, GCT flag + 2 colors (size field 0).
    b.extend_from_slice(&[0x01, 0x00, 0x01, 0x00, 0x80, 0x00, 0x00]);
    // Global color table: 2 entries x 3 bytes.
    b.extend_from_slice(&[0, 0, 0, 0, 0, 0]);
    // Image descriptor: 0x2C, left=0, top=0, 1x1, no local color table.
    b.extend_from_slice(&[0x2C, 0x00, 0x00, 0x00, 0x00, 0x01, 0x00, 0x01, 0x00, 0x00]);
    // LZW minimum code size = 2, then a sub-block whose data contains 0x2C.
    b.push(0x02);
    b.push(0x02); // sub-block length
    b.extend_from_slice(&[0x2C, 0x00]);
    b.push(0x00); // sub-block terminator
    b.push(0x3B); // trailer
    b
}

/// Build a *decodable* static GIF (via the `image` encoder) whose LZW data
/// contains a `0x2C` byte, for the end-to-end `process_image` test. A 32x32
/// patterned image compresses to a LZW stream that includes `0x2C`.
fn encoded_static_gif_with_0x2c_lzw() -> Vec<u8> {
    let w = 32u32;
    let h = 32u32;
    let img = image::RgbaImage::from_fn(w, h, |x, y| {
        image::Rgba([
            ((x * 7) % 256) as u8,
            ((y * 13) % 256) as u8,
            ((x ^ y) % 256) as u8,
            255,
        ])
    });
    let mut buf = Cursor::new(Vec::new());
    img.write_to(&mut buf, image::ImageFormat::Gif).expect("encode gif");
    buf.into_inner()
}

// ============ sniff_mime ============

#[test]
fn sniff_jpeg() {
    assert_eq!(sniff_mime(&[0xFF, 0xD8, 0xFF, 0xFF]).unwrap(), "image/jpeg");
}

#[test]
fn sniff_png() {
    let sig = [0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A];
    assert_eq!(sniff_mime(&sig).unwrap(), "image/png");
}

#[test]
fn sniff_gif() {
    assert_eq!(sniff_mime(b"GIF89a....").unwrap(), "image/gif");
}

#[test]
fn sniff_webp() {
    // RIFF + size + WEBP
    let mut b = b"RIFF".to_vec();
    b.extend_from_slice(&[0, 0, 0, 0]);
    b.extend_from_slice(b"WEBPVP8 ");
    assert_eq!(sniff_mime(&b).unwrap(), "image/webp");
}

#[test]
fn sniff_riff_not_webp() {
    let mut b = b"RIFF".to_vec();
    b.extend_from_slice(&[0, 0, 0, 0]);
    b.extend_from_slice(b"AVI ");
    assert_eq!(sniff_mime(&b), Err(ImageError::Unsupported));
}

#[test]
fn sniff_unknown() {
    assert_eq!(sniff_mime(b"hello world"), Err(ImageError::Unsupported));
}

#[test]
fn sniff_empty() {
    assert_eq!(sniff_mime(b""), Err(ImageError::Unsupported));
}

// ============ process_image ============

#[test]
fn process_png() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("test.png");
    write_image(&path, 100, 50, [10, 20, 30]);

    let img = process_image(&path).unwrap();
    assert_eq!(img.mime_type, "image/png");
    assert_eq!(img.width, 100);
    assert_eq!(img.height, 50);
    // base64 decodes back to a valid PNG signature
    let decoded = base64::engine::general_purpose::STANDARD
        .decode(&img.data)
        .unwrap();
    assert!(decoded.starts_with(&[0x89, 0x50, 0x4E, 0x47]));
}

#[test]
fn process_jpeg() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("test.jpg");
    write_image(&path, 100, 50, [10, 20, 30]);

    let img = process_image(&path).unwrap();
    assert_eq!(img.mime_type, "image/jpeg");
    assert_eq!(img.width, 100);
    assert_eq!(img.height, 50);
    let decoded = base64::engine::general_purpose::STANDARD
        .decode(&img.data)
        .unwrap();
    assert!(decoded.starts_with(&[0xFF, 0xD8, 0xFF]));
}

#[test]
fn process_static_gif() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("test.gif");
    write_image(&path, 100, 50, [10, 20, 30]);

    let img = process_image(&path).unwrap();
    assert_eq!(img.mime_type, "image/gif");
    // re-encoded to PNG
    let decoded = base64::engine::general_purpose::STANDARD
        .decode(&img.data)
        .unwrap();
    assert!(decoded.starts_with(&[0x89, 0x50, 0x4E, 0x47]));
}

#[test]
fn process_animated_gif_rejected() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("animated.gif");
    write_animated_gif(&path, 100, 50);

    assert_eq!(process_image(&path), Err(ImageError::Unsupported));
}

// ============ M-1: GIF block walk ============

#[test]
fn process_static_gif_with_0x2c_lzw_accepted() {
    // M-1 regression: a valid static GIF whose LZW data contains a `0x2C`
    // byte must be accepted (the old byte counter rejected it).
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("static_0x2c.gif");
    let bytes = encoded_static_gif_with_0x2c_lzw();
    // Sanity: the LZW data really does contain a 0x2C byte beyond the single
    // image-descriptor marker (the first 0x2C is the descriptor).
    let first = bytes.iter().position(|&x| x == 0x2C).expect("image descriptor");
    assert!(
        bytes[first + 1..].contains(&0x2C),
        "expected a 0x2C byte in the LZW data"
    );
    std::fs::write(&path, &bytes).unwrap();

    let img = process_image(&path).unwrap();
    assert_eq!(img.mime_type, "image/gif");
    assert_eq!(img.width, 32);
    assert_eq!(img.height, 32);
}

#[test]
fn is_animated_gif_static_with_0x2c_lzw_is_false() {
    let bytes = static_gif_with_0x2c_lzw();
    assert_eq!(is_animated_gif(&bytes), Ok(false));
}

#[test]
fn is_animated_gif_two_frames_is_true() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("two_frames.gif");
    write_animated_gif(&path, 10, 10);
    let bytes = std::fs::read(&path).unwrap();
    assert_eq!(is_animated_gif(&bytes), Ok(true));
}

#[test]
fn is_animated_gif_malformed_is_false() {
    // Truncated header, a bare 0x2C, and empty input all return Ok(false) —
    // the walk never panics and the `image` decoder is the real authority.
    assert_eq!(is_animated_gif(b"GIF8"), Ok(false));
    assert_eq!(is_animated_gif(&[0x2C, 0x2C, 0x2C]), Ok(false));
    assert_eq!(is_animated_gif(b""), Ok(false));
}

#[test]
fn process_bmp_rejected() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("test.bmp");
    // BMP magic: "BM" + size
    std::fs::write(&path, b"BM\x00\x00\x00\x00").unwrap();
    assert_eq!(process_image(&path), Err(ImageError::Unsupported));
}

#[test]
fn process_missing_file() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("nope.png");
    assert!(matches!(process_image(&path), Err(ImageError::Io(_))));
}

#[test]
fn process_empty_file() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("empty.png");
    std::fs::write(&path, b"").unwrap();
    assert_eq!(process_image(&path), Err(ImageError::Unsupported));
}

#[test]
fn process_resizes_large_image() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("large.png");
    write_image(&path, 4000, 2000, [10, 20, 30]);

    let img = process_image(&path).unwrap();
    // 4000x2000 → fit within 2000x2000 → 2000x1000
    assert_eq!(img.width, 2000);
    assert_eq!(img.height, 1000);
}

// ============ estimate_image_tokens ============

#[test]
fn estimate_max_dimension() {
    // 2000*2000 = 4_000_000; /1_048_576 = 3; 1600 + 3*448 = 2944
    assert_eq!(estimate_image_tokens(2000, 2000), 2944);
}

#[test]
fn estimate_one_megapixel() {
    // 1024*1024 = 1_048_576; /1_048_576 = 1; 1600 + 448 = 2048
    assert_eq!(estimate_image_tokens(1024, 1024), 2048);
}

#[test]
fn estimate_sub_megapixel() {
    // 500*500 = 250_000; /1_048_576 = 0; 1600
    assert_eq!(estimate_image_tokens(500, 500), 1600);
}

#[test]
fn estimate_tiny() {
    assert_eq!(estimate_image_tokens(1, 1), 1600);
}

#[test]
fn estimate_zero() {
    assert_eq!(estimate_image_tokens(0, 0), 1600);
}
