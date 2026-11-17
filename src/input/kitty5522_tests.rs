//! Unit tests for the 5522 (OSC) frame parser and state machine.
//!
//! base64 values used here:
//! - `b64("abc")` = `YWJj`
//! - `b64("image/png")` = `aW1hZ2UvcG5n`
//! - `b64("image/png text/plain")` = `aW1hZ2UvcG5nIHRleHQvcGxhaW4=`
//! - `b64(".")` = `Lg==`
//! - `b64("hello")` = `aGVsbG8=`
//! - `b64("Paste event")` = `UGFzdGUgZXZlbnQ=`

use super::kitty5522::*;

#[test]
fn parse_ok_frame_with_pw() {
    // `type=read:status=OK:pw=<b64("abc")>`
    let frame = parse(b"type=read:status=OK:pw=YWJj").unwrap();
    assert_eq!(frame.status, "OK");
    assert_eq!(frame.pw.as_deref(), Some("abc"));
    assert_eq!(frame.mime, None);
    assert_eq!(frame.payload, None);
}

#[test]
fn parse_data_frame_listing() {
    // `type=read:status=DATA:mime=<b64(".")>;<b64("image/png text/plain")>`
    let frame =
        parse(b"type=read:status=DATA:mime=Lg==;aW1hZ2UvcG5nIHRleHQvcGxhaW4=").unwrap();
    assert_eq!(frame.status, "DATA");
    assert_eq!(frame.mime.as_deref(), Some("."));
    assert_eq!(
        frame.payload.as_deref(),
        Some("aW1hZ2UvcG5nIHRleHQvcGxhaW4=")
    );
}

#[test]
fn parse_data_frame_image() {
    // `type=read:status=DATA:mime=<b64("image/png")>;<b64("hello")>`
    let frame = parse(b"type=read:status=DATA:mime=aW1hZ2UvcG5n;aGVsbG8=").unwrap();
    assert_eq!(frame.status, "DATA");
    assert_eq!(frame.mime.as_deref(), Some("image/png"));
    assert_eq!(frame.payload.as_deref(), Some("aGVsbG8="));
}

#[test]
fn parse_done_frame() {
    let frame = parse(b"type=read:status=DONE").unwrap();
    assert_eq!(frame.status, "DONE");
    assert_eq!(frame.mime, None);
    assert_eq!(frame.pw, None);
    assert_eq!(frame.payload, None);
}

#[test]
fn parse_error_frame() {
    let frame = parse(b"type=read:status=EPERM").unwrap();
    assert_eq!(frame.status, "EPERM");
}

#[test]
fn parse_non_read_returns_none() {
    assert!(parse(b"type=write:status=OK").is_none());
    assert!(parse(b"status=OK").is_none());
    assert!(parse(b"").is_none());
}

#[test]
fn parse_bad_base64_mime_returns_none() {
    // A `mime` value that is not valid base64 must be rejected.
    assert!(parse(b"type=read:status=DATA:mime=!!!").is_none());
}

#[test]
fn read_request_shape() {
    let req = read_request("abc", &["image/png".to_string()]);
    // `5522;type=read;name=<b64("Paste event")>;pw=<b64("abc")>;<b64("image/png")>`
    assert!(req.starts_with("\x1b]5522;type=read;name=UGFzdGUgZXZlbnQ=;pw=YWJj;"));
    assert!(req.ends_with("aW1hZ2UvcG5n\x1b\\"));
}

#[test]
fn enable_disable_request_shape() {
    assert_eq!(ENABLE_REQUEST, "\x1b[?5522h");
    assert_eq!(DISABLE_REQUEST, "\x1b[?5522l");
}

#[test]
fn decode_base64_roundtrip() {
    let bytes = decode_base64("aGVsbG8=").unwrap();
    assert_eq!(bytes, b"hello");
    assert!(decode_base64("not valid base64!!!").is_none());
}

#[test]
fn encode_base64_roundtrip() {
    assert_eq!(encode_base64("abc"), "YWJj");
    assert_eq!(encode_base64("image/png"), "aW1hZ2UvcG5n");
}

/// Helper: build a `Pending` state with a pw and deadline.
fn pending(pw: &str, now: std::time::Instant) -> ClipboardState {
    ClipboardState::Pending {
        pw: pw.to_string(),
        reading: false,
        deadline: now + std::time::Duration::from_secs(2),
        data: Vec::new(),
    }
}

#[test]
fn state_machine_ok_stores_pw() {
    let now = std::time::Instant::now();
    let frame = Frame {
        status: "OK".into(),
        mime: None,
        pw: Some("abc".into()),
        payload: None,
    };
    let (state, action) = advance(ClipboardState::Idle, &frame, now);
    assert!(matches!(state, ClipboardState::Pending { .. }));
    assert_eq!(action, ClipboardAction::None);
}

#[test]
fn state_machine_listing_requests_image() {
    let now = std::time::Instant::now();
    let state = pending("abc", now);
    // The MIME listing offers image/png.
    let frame = Frame {
        status: "DATA".into(),
        mime: Some(".".into()),
        pw: None,
        payload: Some("aW1hZ2UvcG5nIHRleHQvcGxhaW4=".into()),
    };
    let (new_state, action) = advance(state, &frame, now);
    match action {
        ClipboardAction::RequestImagePng(request) => {
            assert!(request.contains("aW1hZ2UvcG5n"));
            assert!(request.contains("YWJj"));
        }
        other => panic!("expected RequestImagePng, got {other:?}"),
    }
    // Now `reading` is true.
    assert!(matches!(
        new_state,
        ClipboardState::Pending {
            reading: true, ..
        }
    ));
}

#[test]
fn state_machine_listing_without_image() {
    let now = std::time::Instant::now();
    let state = pending("abc", now);
    // The MIME listing offers only text.
    let frame = Frame {
        status: "DATA".into(),
        mime: Some(".".into()),
        pw: None,
        payload: Some("dGV4dC9wbGFpbg==".into()), // b64("text/plain")
    };
    let (new_state, action) = advance(state, &frame, now);
    assert_eq!(new_state, ClipboardState::Idle);
    assert_eq!(action, ClipboardAction::None);
}

#[test]
fn state_machine_data_chunk_accumulates() {
    let now = std::time::Instant::now();
    // `reading` = true (the read request was sent).
    let state = ClipboardState::Pending {
        pw: "abc".into(),
        reading: true,
        deadline: now + std::time::Duration::from_secs(2),
        data: Vec::new(),
    };
    let frame = Frame {
        status: "DATA".into(),
        mime: Some("image/png".into()),
        pw: None,
        payload: Some("aGVsbG8=".into()),
    };
    let (new_state, action) = advance(state, &frame, now);
    assert_eq!(action, ClipboardAction::None);
    match new_state {
        ClipboardState::Pending { data, .. } => assert_eq!(data, b"hello"),
        other => panic!("expected Pending, got {other:?}"),
    }
}

#[test]
fn state_machine_done_returns_image() {
    let now = std::time::Instant::now();
    let state = ClipboardState::Pending {
        pw: "abc".into(),
        reading: true,
        deadline: now + std::time::Duration::from_secs(2),
        data: b"hello".to_vec(),
    };
    let frame = Frame {
        status: "DONE".into(),
        mime: None,
        pw: None,
        payload: None,
    };
    let (new_state, action) = advance(state, &frame, now);
    assert_eq!(new_state, ClipboardState::Idle);
    assert_eq!(action, ClipboardAction::Image(b"hello".to_vec()));
}

#[test]
fn state_machine_paste_event_done_resets_idle() {
    // The paste event's own DONE (while `reading` is false) resets the state to
    // Idle so the next paste is accepted (not treated as the read response's DONE).
    let now = std::time::Instant::now();
    let state = pending("abc", now); // reading = false
    let frame = Frame {
        status: "DONE".into(),
        mime: None,
        pw: None,
        payload: None,
    };
    let (new_state, action) = advance(state, &frame, now);
    assert_eq!(new_state, ClipboardState::Idle);
    assert_eq!(action, ClipboardAction::None);
}

#[test]
fn state_machine_error_aborts() {
    let now = std::time::Instant::now();
    let state = pending("abc", now);
    let frame = Frame {
        status: "EPERM".into(),
        mime: None,
        pw: None,
        payload: None,
    };
    let (new_state, action) = advance(state, &frame, now);
    assert_eq!(new_state, ClipboardState::Idle);
    assert_eq!(action, ClipboardAction::Error("EPERM".into()));
}

#[test]
fn is_timed_out_helper() {
    let now = std::time::Instant::now();
    let deadline = now + std::time::Duration::from_secs(2);
    let pending = ClipboardState::Pending {
        pw: "abc".into(),
        reading: false,
        deadline,
        data: Vec::new(),
    };
    assert!(!is_timed_out(&pending, now));
    assert!(is_timed_out(&pending, deadline));
    assert!(!is_timed_out(&ClipboardState::Idle, now));
}
