//! Tests for SSE debug capture module.

use std::fs::File;
use std::io::Read;

use crate::sse_debug::init;
use crate::sse_debug::TurnCaptureRef;
use crate::sse_debug::{SseDebugConfig, set_current_turn, current_turn};

#[serial_test::serial]
#[test]
fn sse_debug_init_disabled_when_unset() {
    // Safety: test-only env var manipulation, single-threaded test context.
    unsafe { std::env::set_var("RESPONDAMI_SSE_DEBUG", "") };
    let result = init();
    assert!(result.is_none(), "should be None when env var is empty");
}

#[serial_test::serial]
#[test]
fn sse_debug_init_enabled_with_path() {
    let temp_dir = tempfile::tempdir().unwrap();
    let sse_dir = temp_dir.path().join("sse-debug");
    // Safety: test-only env var manipulation, single-threaded test context.
    unsafe { std::env::set_var("RESPONDAMI_SSE_DEBUG", sse_dir.to_str().unwrap()) };
    let result = init();
    assert!(result.is_some(), "should be Some when env var is set to a path");
    assert!(sse_dir.exists(), "directory should be created");
}

#[test]
fn sse_debug_start_turn_creates_turn_file() {
    let temp_dir = tempfile::tempdir().unwrap();
    let config = SseDebugConfig::new(temp_dir.path().to_path_buf());

    let capture = config.start_turn(Some("session123")).expect("turn file should be created");
    let path = capture.path().clone();

    assert!(path.exists(), "file should exist");
    assert!(path.starts_with(temp_dir.path()), "file should be in temp dir");
    assert!(path.extension().map(|e| e == "log").unwrap_or(false), "file should have .log extension");
    assert!(path.file_name().unwrap().to_string_lossy().starts_with("turn-session123-"), "filename should contain session ID");

    // Verify turn header was written
    let mut contents = String::new();
    File::open(&path).unwrap().read_to_string(&mut contents).unwrap();
    assert!(contents.starts_with("=== TURN START"), "should start with turn header");
    assert!(contents.contains("session=session123"), "header should contain session ID");
}

#[test]
fn sse_debug_start_turn_creates_unique_files() {
    let temp_dir = tempfile::tempdir().unwrap();
    let config = SseDebugConfig::new(temp_dir.path().to_path_buf());

    let capture1 = config.start_turn(Some("session1")).expect("turn 1");
    let capture2 = config.start_turn(Some("session2")).expect("turn 2");

    assert_ne!(capture1.path(), capture2.path(), "each call should produce a unique path");
}

#[test]
fn sse_debug_start_turn_fallback_naming() {
    let temp_dir = tempfile::tempdir().unwrap();
    let config = SseDebugConfig::new(temp_dir.path().to_path_buf());

    let capture = config.start_turn(None).expect("turn file should be created");
    let filename = capture.path().file_name().unwrap().to_string_lossy();

    assert!(filename.starts_with("turn-"), "filename should start with turn-");
    assert!(filename.ends_with(".log"), "filename should end with .log");
    // Fallback format: turn-YYYYMMDD-HHMMSS-NNNN.log
    assert!(filename.contains("-0001.log") || filename.contains("-0002.log"), "should contain sequence number");
}

#[test]
fn turn_capture_write_request_and_response() {
    let temp_dir = tempfile::tempdir().unwrap();
    let config = SseDebugConfig::new(temp_dir.path().to_path_buf());

    let capture = config.start_turn(Some("test-session")).expect("turn file");
    let path = capture.path().clone();

    // Write a request
    capture.write_request(r#"{"model":"llama3","messages":[]}"#);

    // Write response header and bytes
    capture.write_response_header();
    capture.write_response(b"data: {\"choices\":[{\"delta\":{\"content\":\"hello\"}}]}\n\n");

    drop(capture);

    let mut contents = String::new();
    File::open(&path).unwrap().read_to_string(&mut contents).unwrap();

    assert!(contents.starts_with("=== TURN START"), "should start with turn header");
    assert!(contents.contains("--- REQUEST 1"), "should contain request section");
    assert!(contents.contains(r#"{"model":"llama3","messages":[]}"#), "should contain request body");
    assert!(contents.contains("--- RESPONSE 1"), "should contain response section");
    assert!(contents.contains("data: {\"choices\""), "should contain raw SSE data");
    assert!(contents.contains("=== TURN END"), "should end with turn end marker");
}

#[test]
fn turn_capture_drop_writes_end_marker() {
    let temp_dir = tempfile::tempdir().unwrap();
    let config = SseDebugConfig::new(temp_dir.path().to_path_buf());

    let capture = config.start_turn(Some("test-session")).expect("turn file");
    let path = capture.path().clone();
    drop(capture);

    let mut contents = String::new();
    File::open(&path).unwrap().read_to_string(&mut contents).unwrap();
    assert!(contents.contains("=== TURN END"), "should contain turn end marker after drop");
}

#[test]
fn turn_capture_iteration_numbering() {
    let temp_dir = tempfile::tempdir().unwrap();
    let config = SseDebugConfig::new(temp_dir.path().to_path_buf());

    let capture = config.start_turn(Some("test-session")).expect("turn file");
    let path = capture.path().clone();

    capture.write_request("request1");
    capture.write_response_header();
    capture.write_response(b"data: resp1\n\n");
    capture.write_request("request2");
    capture.write_response_header();
    capture.write_response(b"data: resp2\n\n");
    drop(capture);

    let mut contents = String::new();
    File::open(&path).unwrap().read_to_string(&mut contents).unwrap();

    assert!(contents.contains("--- REQUEST 1"), "should have REQUEST 1");
    assert!(contents.contains("--- RESPONSE 1"), "should have RESPONSE 1");
    assert!(contents.contains("--- REQUEST 2"), "should have REQUEST 2");
    assert!(contents.contains("--- RESPONSE 2"), "should have RESPONSE 2");
}

#[test]
fn turn_capture_global_set_and_get() {
    let temp_dir = tempfile::tempdir().unwrap();
    let config = SseDebugConfig::new(temp_dir.path().to_path_buf());

    let capture: TurnCaptureRef = config.start_turn(Some("global-test")).expect("turn file");
    let guard = set_current_turn(capture.clone());

    let retrieved = current_turn();
    assert!(retrieved.is_some(), "should retrieve current turn capture");
    assert_eq!(retrieved.unwrap().path(), capture.path(), "should be same capture");

    drop(guard);
    assert!(current_turn().is_none(), "should be None after guard dropped");
}

#[serial_test::serial]
#[test]
fn sse_debug_init_creates_nested_directory() {
    let temp_dir = tempfile::tempdir().unwrap();
    let nested = temp_dir.path().join("a").join("b").join("c");
    // Safety: test-only env var manipulation, single-threaded test context.
    unsafe { std::env::set_var("RESPONDAMI_SSE_DEBUG", nested.to_str().unwrap()) };
    let result = init();
    assert!(result.is_some(), "should create nested directories");
    assert!(nested.exists(), "nested directory should be created");
}
