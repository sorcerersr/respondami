//! SSE debug capture — logs full turn communication to per-turn files.
//!
//! Activated via `RESPONDAMI_SSE_DEBUG` environment variable:
//! - Unset or empty → disabled (zero overhead)
//! - `1` → writes to `.respondami/sse-debug/` in CWD
//! - `/custom/path` → writes to specified directory
//!
//! Each agentic turn gets one file named `turn-{session_id}-{seq:04}.log`
//! containing both request (JSON body) and response (raw SSE bytes) for
//! every iteration within that turn.

use std::fs::{self, File};
use std::io::Write;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};

/// Configuration for SSE debug capture.
#[derive(Debug)]
pub struct SseDebugConfig {
    dir: PathBuf,
    counter: AtomicU32,
}

impl SseDebugConfig {
    /// Create a new debug config for the given output directory.
    ///
    /// # Panics
    ///
    /// Does not panic. The directory must already exist.
    #[must_use]
    pub fn new(dir: PathBuf) -> Self {
        Self {
            dir,
            counter: AtomicU32::new(1),
        }
    }

    /// Open a new turn-scoped capture file.
    ///
    /// Returns `Some(TurnCaptureRef)` on success. Errors during file creation are
    /// silently ignored — SSE debug must never affect normal operation.
    pub fn start_turn(&self, session_id: Option<&str>) -> Option<Arc<TurnCapture>> {
        let seq = self.counter.fetch_add(1, Ordering::Relaxed);
        let filename = if let Some(id) = session_id {
            format!("turn-{}-{seq:04}.log", sanitize_session_id(id))
        } else {
            let now = chrono::Local::now().format("%Y%m%d-%H%M%S");
            format!("turn-{now}-{seq:04}.log")
        };
        let path = self.dir.join(&filename);

        let file = match File::create(&path) {
            Ok(f) => f,
            Err(e) => {
                tracing::debug!(error = %e, "SSE debug: failed to create capture file");
                return None;
            }
        };

        let capture = Arc::new(TurnCapture {
            file: Mutex::new(file),
            path,
            request_iteration: AtomicU32::new(0),
            response_iteration: AtomicU32::new(0),
        });

        // Write turn header
        let now = chrono::Local::now().format("%Y-%m-%d %H:%M:%S");
        let header = match session_id {
            Some(id) => format!("=== TURN START ({now}) session={id} ===\n"),
            None => format!("=== TURN START ({now}) ===\n"),
        };
        let mut guard = capture.file.lock().expect("TurnCapture file lock");
        if let Err(e) = guard.write_all(header.as_bytes()) {
            tracing::debug!(error = %e, "SSE debug: write header failed");
        }
        drop(guard);

        Some(capture)
    }
}

/// Sanitize a session ID for use in filenames.
fn sanitize_session_id(id: &str) -> String {
    id.chars()
        .map(|c| match c {
            'a'..='z' | 'A'..='Z' | '0'..='9' | '-' | '_' => c,
            _ => '_',
        })
        .take(32)
        .collect()
}

/// Initialize SSE debug capture from the `RESPONDAMI_SSE_DEBUG` environment variable.
///
/// Returns `Some(SseDebugConfig)` if the environment variable is set and non-empty.
/// Creates the output directory if it does not exist.
pub fn init() -> Option<SseDebugConfig> {
    let value = std::env::var("RESPONDAMI_SSE_DEBUG").ok()?;
    if value.is_empty() {
        return None;
    }

    let dir = if value == "1" {
        std::env::current_dir()
            .ok()
            .unwrap_or_else(|| PathBuf::from("."))
            .join(".respondami")
            .join("sse-debug")
    } else {
        PathBuf::from(&value)
    };

    if let Err(e) = fs::create_dir_all(&dir) {
        tracing::warn!(error = %e, dir = %dir.display(), "SSE debug: failed to create directory");
        return None;
    }

    tracing::info!(dir = %dir.display(), "SSE debug capture enabled");
    Some(SseDebugConfig {
        dir,
        counter: AtomicU32::new(1),
    })
}

// ---------------------------------------------------------------------------
// Turn-scoped capture
// ---------------------------------------------------------------------------

/// Turn-scoped capture that writes both request and response to a single file.
///
/// Thread-safe via internal mutex — request and response may write concurrently
/// from different async tasks.
pub struct TurnCapture {
    file: Mutex<File>,
    path: PathBuf,
    /// Request iteration counter within the turn.
    request_iteration: AtomicU32,
    /// Response iteration counter within the turn.
    response_iteration: AtomicU32,
}

impl std::fmt::Debug for TurnCapture {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TurnCapture")
            .field("path", &self.path)
            .finish_non_exhaustive()
    }
}

/// Shared reference to a turn capture. `Arc` allows cloning across spawned tasks.
pub type TurnCaptureRef = Arc<TurnCapture>;

/// Global holder for the current turn's capture.
/// the request body. Cleared at the end of the turn.
static CURRENT_TURN: Mutex<Option<TurnCaptureRef>> = Mutex::new(None);

/// Set the current turn capture. Returns a guard that clears on drop.
///
/// Used to make the capture accessible from `stream_chat_inner` without
/// modifying the `ProviderTrait` interface.
pub fn set_current_turn(capture: TurnCaptureRef) -> TurnCaptureGuard {
    let mut guard = CURRENT_TURN.lock().expect("CURRENT_TURN lock");
    *guard = Some(capture);
    TurnCaptureGuard
}

/// Guard that clears the current turn capture on drop.
pub struct TurnCaptureGuard;

impl std::fmt::Debug for TurnCaptureGuard {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TurnCaptureGuard").finish()
    }
}

impl Drop for TurnCaptureGuard {
    fn drop(&mut self) {
        let mut guard = CURRENT_TURN.lock().expect("CURRENT_TURN lock");
        *guard = None;
    }
}

/// Get the current turn capture, if one is active.
pub fn current_turn() -> Option<TurnCaptureRef> {
    CURRENT_TURN
        .lock()
        .expect("CURRENT_TURN lock")
        .clone()
}

impl TurnCapture {
    /// Path to the capture file.
    #[must_use]
    pub fn path(&self) -> &PathBuf {
        &self.path
    }

    /// Write a request section header and body.
    ///
    /// Errors are silently logged at DEBUG level — capture must never
    /// affect normal streaming operation.
    pub fn write_request(&self, body: &str) {
        let iteration = self.request_iteration.fetch_add(1, Ordering::Relaxed) + 1;
        let now = chrono::Local::now().format("%Y-%m-%dT%H:%M:%S.%3f");
        let header = format!("--- REQUEST {iteration} ({now}) ---\n");

        let mut guard = self.file.lock().expect("TurnCapture file lock");
        if let Err(e) = guard.write_all(header.as_bytes()) {
            tracing::debug!(error = %e, path = %self.path.display(), "SSE debug: write request header failed");
        }
        if let Err(e) = guard.write_all(body.as_bytes()) {
            tracing::debug!(error = %e, path = %self.path.display(), "SSE debug: write request body failed");
        }
        if let Err(e) = guard.write_all(b"\n\n") {
            tracing::debug!(error = %e, path = %self.path.display(), "SSE debug: write request trailing newlines failed");
        }
        if let Err(e) = guard.flush() {
            tracing::debug!(error = %e, path = %self.path.display(), "SSE debug: flush failed");
        }
    }

    /// Write a response section header.
    ///
    /// Called once per iteration before the raw SSE bytes are written.
    pub fn write_response_header(&self) {
        let iteration = self.response_iteration.fetch_add(1, Ordering::Relaxed) + 1;
        let now = chrono::Local::now().format("%Y-%m-%dT%H:%M:%S.%3f");
        let header = format!("--- RESPONSE {iteration} ({now}) ---\n");

        let mut guard = self.file.lock().expect("TurnCapture file lock");
        if let Err(e) = guard.write_all(header.as_bytes()) {
            tracing::debug!(error = %e, path = %self.path.display(), "SSE debug: write response header failed");
        }
        if let Err(e) = guard.flush() {
            tracing::debug!(error = %e, path = %self.path.display(), "SSE debug: flush failed");
        }
    }

    /// Write raw SSE response bytes.
    ///
    /// Errors are silently logged at DEBUG level — capture must never
    /// affect normal streaming operation.
    pub fn write_response(&self, data: &[u8]) {
        let mut guard = self.file.lock().expect("TurnCapture file lock");
        if let Err(e) = guard.write_all(data) {
            tracing::debug!(error = %e, path = %self.path.display(), "SSE debug: write response failed");
        }
        if let Err(e) = guard.flush() {
            tracing::debug!(error = %e, path = %self.path.display(), "SSE debug: flush failed");
        }
    }
}

impl Drop for TurnCapture {
    fn drop(&mut self) {
        let now = chrono::Local::now().format("%Y-%m-%d %H:%M:%S");
        let footer = format!("=== TURN END ({now}) ===\n");
        let mut guard = self.file.lock().expect("TurnCapture file lock");
        if let Err(e) = guard.write_all(footer.as_bytes()) {
            tracing::debug!(error = %e, path = %self.path.display(), "SSE debug: write turn end failed");
        }
        if let Err(e) = guard.flush() {
            tracing::debug!(error = %e, path = %self.path.display(), "SSE debug: flush on drop failed");
        }
        tracing::debug!(path = %self.path.display(), "SSE debug: turn capture closed");
    }
}
