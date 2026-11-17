//! Kitty 5522 clipboard protocol: frame parsing and the read-request builder.
//!
//! 5522 is an **OSC** protocol (an extension of OSC 52), not a DCS protocol.
//! Every frame has the form:
//!
//! ```text
//! ESC ] 5522;metadata;payload ST
//! ```
//!
//! where `metadata` is a colon-separated list of `key=value` pairs and
//! `payload` is base64 data (optional). The `mime`, `name`, and `pw` metadata
//! values are base64-encoded UTF-8.
//!
//! The app enables the paste-event mode with `CSI ? 5522 h`. When the user
//! triggers a paste, the terminal sends a sequence of frames:
//!
//! 1. `5522;type=read:status=OK:pw=<b64(one-time password)>`
//! 2. `5522;type=read:status=DATA:mime=<b64('.')>;<b64(space-separated MIME types)>`
//! 3. `5522;type=read:status=DONE`
//!
//! The app then requests the image data without a permission prompt:
//!
//! ```text
//! 5522;type=read;name=<b64('Paste event')>;pw=<b64(one-time pw)>;<b64('image/png')>
//! ```
//!
//! The terminal replies with `status=OK`, one or more `status=DATA` frames
//! (chunked, ≤4096 bytes each, base64), and `status=DONE`.
//!
//! See <https://sw.kovidgoyal.net/kitty/clipboard/#5522> and the Kitty /
//! Ghostty source (`kitty/clipboard.py`, `src/terminal/kitty/clipboard.zig`).

use base64::Engine;

/// A parsed 5522 frame.
///
/// All frames carry `type=read` and a `status`. The `mime` and `pw` metadata
/// are decoded from base64; the `payload` is the raw base64 string (decoded by
/// the state machine as needed).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Frame {
    /// The status: `OK`, `DATA`, `DONE`, or an error code (`ENOSYS`, `EPERM`,
    /// `EBUSY`, `EINVAL`).
    pub status: String,
    /// The MIME type (decoded from base64), present on `DATA` frames.
    pub mime: Option<String>,
    /// The one-time password (decoded from base64), present on `OK` frames.
    pub pw: Option<String>,
    /// The base64 payload (the bytes after the second `;`), if any.
    pub payload: Option<String>,
}

/// The special MIME type that requests the list of available types.
pub const TARGETS_MIME: &str = ".";

/// The human name to send with a read request that uses the one-time password.
/// Per the spec, `name` SHOULD be set to `Paste event` when using the one-time
/// password from a paste event.
pub const PASTE_EVENT_NAME: &str = "Paste event";

/// Parse a raw 5522 payload (the bytes between `ESC ] 5522` and the ST).
///
/// Returns `None` for anything that is not a well-formed 5522 read frame, so a
/// malformed frame is ignored (debug-logged by the caller) and never crashes
/// the loop.
#[must_use]
pub fn parse(payload: &[u8]) -> Option<Frame> {
    let s = std::str::from_utf8(payload).ok()?;
    // Split into metadata and (optional) payload at the first `;`.
    let (metadata, epayload) = match s.find(';') {
        Some(idx) => (&s[..idx], s[idx + 1..].to_string()),
        None => (s, String::new()),
    };
    let mut status = String::new();
    let mut mime: Option<String> = None;
    let mut pw: Option<String> = None;
    let mut is_read = false;
    for record in metadata.split(':') {
        let (k, v) = record.split_once('=')?;
        match k {
            "type" if v == "read" => is_read = true,
            "status" => status = v.to_string(),
            "mime" => {
                let decoded = decode_base64(v)?;
                mime = Some(String::from_utf8(decoded).ok()?);
            }
            "pw" => {
                let decoded = decode_base64(v)?;
                pw = Some(String::from_utf8(decoded).ok()?);
            }
            // `name`, `id`, `loc` are not needed by the app.
            _ => {}
        }
    }
    if !is_read || status.is_empty() {
        return None;
    }
    let payload = if epayload.is_empty() {
        None
    } else {
        Some(epayload)
    };
    Some(Frame {
        status,
        mime,
        pw,
        payload,
    })
}

/// Build the 5522 read request for the given MIME types, using the one-time
/// password from a paste event.
///
/// The format is `5522;type=read;name=<b64('Paste event')>;pw=<b64>;<b64 mimes>`
/// where `mimes` is a space-separated list (e.g. `image/png`).
#[must_use]
pub fn read_request(pw: &str, mimes: &[String]) -> String {
    let name_b64 = encode_base64(PASTE_EVENT_NAME);
    let pw_b64 = encode_base64(pw);
    let mimes_b64 = encode_base64(&mimes.join(" "));
    format!(
        "\x1b]5522;type=read;name={name_b64};pw={pw_b64};{mimes_b64}\x1b\\"
    )
}

/// The request string the app sends to enable the 5522 paste-event mode.
pub const ENABLE_REQUEST: &str = "\x1b[?5522h";

/// The request string the app sends to disable the 5522 mode on shutdown.
pub const DISABLE_REQUEST: &str = "\x1b[?5522l";

/// Decode a base64 string to raw bytes.
#[must_use]
pub fn decode_base64(b64: &str) -> Option<Vec<u8>> {
    base64::engine::general_purpose::STANDARD.decode(b64).ok()
}

/// Encode a UTF-8 string to base64 (standard alphabet, with padding).
#[must_use]
pub fn encode_base64(s: &str) -> String {
    base64::engine::general_purpose::STANDARD.encode(s.as_bytes())
}

/// The 5522 clipboard state machine: tracks the in-flight read request and
/// accumulates chunked image data.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum ClipboardState {
    /// No read request is in flight.
    #[default]
    Idle,
    /// A read request is in flight (or the paste event's MIME listing is being
    /// processed). `pw` is the one-time password from the paste event; `data`
    /// holds the decoded image bytes accumulated from `DATA` chunks; `reading`
    /// is true once the read request has been sent (so the paste event's own
    /// `DONE` is not mistaken for the read response's `DONE`).
    Pending {
        /// The one-time password from the paste event (used to build the read
        /// request when the MIME listing arrives).
        pw: String,
        /// True once the read request has been sent and we are waiting for the
        /// read response's `DONE`.
        reading: bool,
        /// The deadline for the terminal's reply.
        deadline: std::time::Instant,
        /// Decoded image bytes accumulated from `DATA` chunks.
        data: Vec<u8>,
    },
}

/// The action the event loop must take in response to a 5522 frame.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ClipboardAction {
    /// A paste event arrived offering `image/png`: send the read request.
    /// Carries the request string to send.
    RequestImagePng(String),
    /// The read request completed: the decoded image bytes.
    Image(Vec<u8>),
    /// The read request failed (error status or timeout).
    Error(String),
    /// Nothing to do.
    None,
}

/// Advance the 5522 state machine with a parsed frame and the current time.
///
/// Returns the new state and the action the event loop must take. The state
/// machine is pure (no I/O); the caller is responsible for sending the read
/// request and attaching the image.
#[must_use]
pub fn advance(
    state: ClipboardState,
    frame: &Frame,
    now: std::time::Instant,
) -> (ClipboardState, ClipboardAction) {
    match state {
        // Paste event: the terminal offers a one-time password. Store it and
        // wait for the MIME listing (a DATA frame with mime=`.`).
        ClipboardState::Idle if frame.status == "OK" => {
            if let Some(pw) = &frame.pw {
                (
                    ClipboardState::Pending {
                        pw: pw.clone(),
                        reading: false,
                        deadline: now + std::time::Duration::from_secs(2),
                        data: Vec::new(),
                    },
                    ClipboardAction::None,
                )
            } else {
                (ClipboardState::Idle, ClipboardAction::None)
            }
        }
        // The MIME listing (mime=`.`): check for image/png and request it.
        ClipboardState::Pending { pw, deadline, .. }
            if frame.status == "DATA" && frame.mime.as_deref() == Some(TARGETS_MIME) =>
        {
            if let Some(payload) = &frame.payload
                && let Some(mimes) = decode_base64(payload).and_then(|b| String::from_utf8(b).ok())
            {
                let available: Vec<String> =
                    mimes.split_whitespace().map(ToString::to_string).collect();
                if available.iter().any(|m| m == "image/png") {
                    let request = read_request(&pw, &["image/png".to_string()]);
                    return (
                        ClipboardState::Pending {
                            pw,
                            reading: true,
                            deadline,
                            data: Vec::new(),
                        },
                        ClipboardAction::RequestImagePng(request),
                    );
                }
            }
            (ClipboardState::Idle, ClipboardAction::None)
        }
        // A data chunk for the image: accumulate it (only while reading).
        ClipboardState::Pending { pw, reading: true, deadline, mut data } if frame.status == "DATA"
            && frame.mime.as_deref() == Some("image/png") =>
        {
            if let Some(payload) = &frame.payload
                && let Some(chunk) = decode_base64(payload)
            {
                data.extend(chunk);
            }
            (
                ClipboardState::Pending { pw, reading: true, deadline, data },
                ClipboardAction::None,
            )
        }
        // Done: the read request completed (only while reading; the paste
        // event's own DONE is ignored).
        ClipboardState::Pending { reading: true, data, .. } if frame.status == "DONE" => {
            if data.is_empty() {
                (ClipboardState::Idle, ClipboardAction::None)
            } else {
                (ClipboardState::Idle, ClipboardAction::Image(data))
            }
        }
        // The paste event's own DONE (reading=false): the paste event completed
        // without an image request. Reset to Idle so the next paste is accepted.
        ClipboardState::Pending { reading: false, .. } if frame.status == "DONE" => (
            ClipboardState::Idle,
            ClipboardAction::None,
        ),
        // An error status: abort.
        ClipboardState::Pending { .. } if frame.status != "DONE" => (
            ClipboardState::Idle,
            ClipboardAction::Error(frame.status.clone()),
        ),
        // Ignore frames that don't match the current state (including the
        // paste event's own DONE, which arrives while `reading` is false).
        other => (other, ClipboardAction::None),
    }
}

/// Check whether a pending request has timed out. Returns `true` if the
/// deadline has passed and the state should be cleared.
#[must_use]
pub fn is_timed_out(state: &ClipboardState, now: std::time::Instant) -> bool {
    match state {
        ClipboardState::Pending { deadline, .. } => now >= *deadline,
        ClipboardState::Idle => false,
    }
}
