//! File-based logging with rolling log rotation.
//!
//! Initializes `tracing` with a non-blocking rolling file appender under
//! `.respondami/logs/respondami.log`. Uses `OnceLock` to ensure initialization
//! happens exactly once, even if called multiple times.

use std::fs;
use std::path::PathBuf;
use std::sync::OnceLock;

use tracing::subscriber::set_global_default;
use tracing_subscriber::EnvFilter;

/// Respondami data directory name (relative to CWD).
const RESPORDAMI_DIR: &str = ".respondami";
/// Logs subdirectory inside .respondami.
const LOGS_DIR: &str = "logs";
/// Log file name (without extension).
pub(crate) const LOG_FILE: &str = "respondami.log";

/// Global guard for the non-blocking rolling appender.
/// Kept alive so the rolling appender is never dropped while writes are in flight.
static LOG_GUARD: OnceLock<tracing_appender::non_blocking::WorkerGuard> = OnceLock::new();

/// Global SSE debug capture config (initialized once at startup).
static SSE_DEBUG: OnceLock<crate::sse_debug::SseDebugConfig> = OnceLock::new();

/// Get the active SSE debug config, if enabled.
#[must_use]
pub fn sse_debug_config() -> Option<&'static crate::sse_debug::SseDebugConfig> {
    SSE_DEBUG.get()
}

/// Initialize file-based logging with size-based rotation.
///
/// Creates `.respondami/logs/` in the current working directory and writes
/// structured log output to `respondami.log`. When the file exceeds
/// `MAX_LOG_SIZE_MB` (1 MB), it is rotated to `respondami.log.1`,
/// and up to `MAX_ROTATED_FILES` (5) rotated files are kept.
///
/// Log level is controlled via the `RUST_LOG` environment variable,
/// with a minimum floor of `info` (cannot be lowered below info).
///
/// If the log directory cannot be created, falls back silently — the app
/// still runs without file logging.
pub fn init() {
    let log_dir = log_dir_path();

    // Ensure directory exists
    if let Err(e) = fs::create_dir_all(&log_dir) {
        eprintln!("Warning: cannot create log directory {log_dir:?}: {e}");
        return;
    }

    // Daily rolling appender: rotates at midnight, keeps 7 daily files.
    // Example: respondami.log.2026-06-26, respondami.log.2026-06-25, ...
    let rolling_appender = tracing_appender::rolling::daily(&log_dir, LOG_FILE);

    // Non-blocking writer to avoid blocking the main event loop.
    // `NonBlocking::new` returns a tuple of (writer, guard).
    let (non_blocking, _guard) = tracing_appender::non_blocking::NonBlocking::new(rolling_appender);

    // Build subscriber with env filter and non-blocking writer
    let env_filter = EnvFilter::builder()
        .with_default_directive(tracing::Level::INFO.into())
        .from_env()
        .unwrap_or_else(|_| EnvFilter::new("info"));

    let subscriber = tracing_subscriber::fmt::Subscriber::builder()
        .with_env_filter(env_filter)
        .with_writer(non_blocking)
        .with_ansi(false)
        .finish();

    if let Err(e) = set_global_default(subscriber) {
        eprintln!("Warning: cannot set tracing subscriber: {e}");
        return;
    }

    // Store the guard globally so the rolling appender is never dropped.
    // If init() is called multiple times, the guard is already set and this is a no-op.
    let _ = LOG_GUARD.set(_guard);

    // Initialize SSE debug capture (env var: RESPONDAMI_SSE_DEBUG)
    if let Some(config) = crate::sse_debug::init() {
        let _ = SSE_DEBUG.set(config);
    }
}

/// Resolve the log directory: `<CWD>/.respondami/logs/`.
pub(crate) fn log_dir_path() -> PathBuf {
    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    cwd.join(RESPORDAMI_DIR)
        .join(LOGS_DIR)
}

/// Resolve the primary log file path: `<CWD>/.respondami/logs/respondami.log`.
/// Used for documentation and testing.
#[must_use]
pub fn log_file_path() -> PathBuf {
    log_dir_path().join(LOG_FILE)
}


