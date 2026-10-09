use std::io::Write;
use std::sync::{Arc, Mutex};

use crate::logging::*;

#[test]
fn log_dir_path_includes_respondami_dir() {
    let path = log_dir_path();
    assert!(path.to_string_lossy().contains(".respondami"));
    assert!(path.to_string_lossy().contains("logs"));
}

#[test]
fn log_file_path_includes_respondami_dir_and_logs() {
    let path = log_file_path();
    assert!(path.to_string_lossy().contains(".respondami"));
    assert!(path.to_string_lossy().contains("logs"));
    assert_eq!(path.file_name().map(|n| n.to_string_lossy().to_string()), Some(LOG_FILE.to_string()));
}

#[test]
fn log_file_path_is_inside_log_dir() {
    let dir = log_dir_path();
    let file = log_file_path();
    assert!(file.starts_with(&dir));
}

// ---- panic hook tests ----

// The global panic hook is process-wide, so tests that swap it must not run
// in parallel. This lock serializes them.
static PANIC_HOOK_LOCK: std::sync::LazyLock<std::sync::Mutex<()>> =
    std::sync::LazyLock::new(|| std::sync::Mutex::new(()));

/// A `tracing_subscriber::MakeWriter` that appends every byte to a shared
/// buffer, so a test can assert on what the subscriber wrote.
#[derive(Debug, Clone)]
struct SharedWriter {
    buf: Arc<Mutex<Vec<u8>>>,
}

impl Write for SharedWriter {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.buf.lock().unwrap().extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for SharedWriter {
    type Writer = SharedWriter;

    fn make_writer(&'a self) -> Self::Writer {
        SharedWriter {
            buf: self.buf.clone(),
        }
    }
}

#[test]
fn format_panic_info_includes_message_and_location() {
    let _guard = PANIC_HOOK_LOCK.lock().unwrap();
    // Capture the formatted message via a temporary hook, then restore the
    // default hook. This tests the pure formatting logic without needing a
    // live tracing subscriber.
    let captured: Arc<Mutex<Option<String>>> = Arc::new(Mutex::new(None));
    let hook_buf = captured.clone();
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        *hook_buf.lock().unwrap() = Some(format_panic_info(info));
    }));

    let result = std::panic::catch_unwind(|| panic!("test panic payload"));
    std::panic::set_hook(Box::new(default_hook));

    assert!(result.is_err(), "panic should have been caught");
    let msg = captured
        .lock()
        .unwrap()
        .take()
        .expect("hook should have captured the message");
    assert!(
        msg.contains("test panic payload"),
        "should contain the payload: {msg}"
    );
    assert!(
        msg.contains("panicked at"),
        "should contain the location: {msg}"
    );
    assert!(
        msg.contains("logging_tests.rs"),
        "should name the file where the panic occurred: {msg}"
    );
    assert!(
        msg.contains("backtrace"),
        "should include a backtrace section: {msg}"
    );
}

#[test]
fn install_panic_hook_logs_panic_to_subscriber() {
    let _guard = PANIC_HOOK_LOCK.lock().unwrap();
    // Install the real hook and verify it emits a tracing record containing
    // the payload and location. A scoped subscriber (thread-local) captures
    // the record without touching the process-global default.
    let buf = Arc::new(Mutex::new(Vec::<u8>::new()));
    let writer = SharedWriter {
        buf: buf.clone(),
    };
    let subscriber = tracing_subscriber::fmt::Subscriber::builder()
        .with_writer(writer)
        .with_ansi(false)
        .finish();
    let guard = tracing::subscriber::set_default(subscriber);

    install_panic_hook();

    let result = std::panic::catch_unwind(|| panic!("hook log payload"));
    assert!(result.is_err(), "panic should have been caught");
    drop(guard);

    let lock_guard = buf.lock().unwrap();
    let text = String::from_utf8_lossy(&lock_guard);
    assert!(
        text.contains("hook log payload"),
        "should log the payload: {text}"
    );
    assert!(
        text.contains("panicked at"),
        "should log the location: {text}"
    );
}
