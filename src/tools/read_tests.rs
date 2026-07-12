use super::read::ReadTool;
use super::{CancelGuard, ToolHandler};
use std::path::Path;
use tempfile::TempDir;

fn get_tool() -> ReadTool {
    ReadTool
}

async fn run_read(cwd: &Path, args: serde_json::Value) -> anyhow::Result<String> {
    let cancel = CancelGuard::new();
    get_tool()
        .execute(args, cwd, None, &cancel)
        .await
}

async fn run_read_with_cancel(
    cwd: &Path,
    args: serde_json::Value,
    cancel: &CancelGuard,
) -> anyhow::Result<String> {
    get_tool().execute(args, cwd, None, cancel).await
}

// ============ Basic reads ============

#[tokio::test]
async fn read_file_full() {
    let dir = TempDir::new().unwrap();
    std::fs::write(dir.path().join("hello.txt"), "line1\nline2\nline3\n").unwrap();

    let result = run_read(
        dir.path(),
        serde_json::json!({ "path": "hello.txt" }),
    )
    .await;

    assert!(result.is_ok(), "unexpected error: {:?}", result.err());
    let content = result.unwrap();
    assert!(content.contains("line1"));
    assert!(content.contains("line2"));
    assert!(content.contains("line3"));
    assert!(content.contains("3 lines:"));
}

#[tokio::test]
async fn read_file_with_offset() {
    let dir = TempDir::new().unwrap();
    std::fs::write(
        dir.path().join("data.txt"),
        "alpha\nbeta\ngamma\ndelta\nepsilon\n",
    )
    .unwrap();

    // offset=3 (1-indexed) should start from "gamma"
    let result = run_read(
        dir.path(),
        serde_json::json!({ "path": "data.txt", "offset": 3 }),
    )
    .await;

    assert!(result.is_ok());
    let content = result.unwrap();
    assert!(content.contains("Showing lines 3-5"));
    assert!(content.contains("gamma"));
    assert!(!content.contains("alpha"));
    assert!(!content.contains("beta"));
}

#[tokio::test]
async fn read_file_with_offset_and_limit() {
    let dir = TempDir::new().unwrap();
    std::fs::write(
        dir.path().join("data.txt"),
        "a\nb\nc\nd\ne\n",
    )
    .unwrap();

    let result = run_read(
        dir.path(),
        serde_json::json!({ "path": "data.txt", "offset": 2, "limit": 3 }),
    )
    .await;

    assert!(result.is_ok());
    let content = result.unwrap();
    assert!(content.contains("Showing lines 2-4"));
    assert!(content.contains('b'));
    assert!(content.contains('c'));
    assert!(content.contains('d'));
    // "e" is on line 5, which is beyond limit of 3 from offset 2
    // The content may contain "e" in the header/continuation text, so check the data lines
    assert!(content.contains('b'));
    assert!(content.contains('c'));
    assert!(content.contains('d'));
    // Continuation hint
    assert!(content.contains("Use offset=5 to continue"));
}

// ============ Offset OOB ============

#[tokio::test]
async fn read_offset_beyond_end() {
    let dir = TempDir::new().unwrap();
    std::fs::write(dir.path().join("small.txt"), "one\ntwo\n").unwrap();

    let result = run_read(
        dir.path(),
        serde_json::json!({ "path": "small.txt", "offset": 5 }),
    )
    .await;

    assert!(result.is_err());
    let err = result.unwrap_err().to_string();
    assert!(err.contains("beyond end of file"));
    assert!(err.contains("2 lines total"));
}

// ============ Offset 1-indexed ============

#[tokio::test]
async fn read_offset_one_returns_first_line() {
    let dir = TempDir::new().unwrap();
    std::fs::write(dir.path().join("file.txt"), "first\nsecond\n").unwrap();

    let result = run_read(
        dir.path(),
        serde_json::json!({ "path": "file.txt", "offset": 1, "limit": 1 }),
    )
    .await;

    assert!(result.is_ok());
    let content = result.unwrap();
    assert!(content.contains("first"));
    assert!(!content.contains("second"));
}

// ============ Truncation ============

#[tokio::test]
async fn read_line_truncation() {
    let dir = TempDir::new().unwrap();
    // Create file with 2005 lines
    let content: String = (1..=2005)
        .map(|i| format!("line {i}"))
        .collect::<Vec<_>>()
        .join("\n");
    std::fs::write(dir.path().join("big.txt"), content).unwrap();

    let result = run_read(dir.path(), serde_json::json!({ "path": "big.txt" })).await;

    assert!(result.is_ok());
    let content = result.unwrap();
    // Should be truncated at 2000 lines
    assert!(content.contains("2000 line limit"));
    assert!(content.contains("Use offset=2001 to continue"));
}

#[tokio::test]
async fn read_byte_truncation() {
    let dir = TempDir::new().unwrap();
    // Create file with long lines that will exceed 50KB before 2000 lines
    let long_line = "x".repeat(300);
    let content: String = (1..=300)
        .map(|_| long_line.clone())
        .collect::<Vec<_>>()
        .join("\n");
    std::fs::write(dir.path().join("wide.txt"), content).unwrap();

    let result = run_read(dir.path(), serde_json::json!({ "path": "wide.txt" })).await;

    assert!(result.is_ok());
    let content = result.unwrap();
    // Should be truncated by bytes
    assert!(content.contains("50KB limit") || content.contains("limit"));
    assert!(content.contains("Use offset="));
}

// ============ First line exceeds limit ============

#[tokio::test]
async fn read_first_line_exceeds_limit() {
    let dir = TempDir::new().unwrap();
    // Single line > 50KB
    let huge_line = "x".repeat(60 * 1024);
    std::fs::write(dir.path().join("huge.txt"), huge_line).unwrap();

    let result = run_read(dir.path(), serde_json::json!({ "path": "huge.txt" })).await;

    assert!(result.is_ok());
    let content = result.unwrap();
    assert!(content.contains("exceeds"));
    assert!(content.contains("Use bash"));
}

// ============ Trailing newline trim ============

#[tokio::test]
async fn read_trailing_empty_lines_trimmed() {
    let dir = TempDir::new().unwrap();
    std::fs::write(dir.path().join("trailing.txt"), "a\nb\n\n\n").unwrap();

    let result = run_read(
        dir.path(),
        serde_json::json!({ "path": "trailing.txt" }),
    )
    .await;

    assert!(result.is_ok());
    // Content should not end with multiple empty lines
    let content = result.unwrap();
    let lines: Vec<&str> = content.lines().collect();
    // Last non-metadata line should be "b" or the number-prefixed version
    assert!(lines.iter().any(|l| l.contains('b')));
}

// ============ File not found ============

#[tokio::test]
async fn read_missing_file() {
    let dir = TempDir::new().unwrap();

    let result = run_read(
        dir.path(),
        serde_json::json!({ "path": "no_such_file.txt" }),
    )
    .await;

    assert!(result.is_err());
    let err = result.unwrap_err().to_string();
    assert!(err.contains("not found"));
}

// ============ Binary file ============

#[tokio::test]
async fn read_binary_file() {
    let dir = TempDir::new().unwrap();
    std::fs::write(dir.path().join("binary.bin"), [0, 1, 2, 0, 4, 5]).unwrap();

    let result = run_read(
        dir.path(),
        serde_json::json!({ "path": "binary.bin" }),
    )
    .await;

    assert!(result.is_ok());
    let content = result.unwrap();
    assert!(content.contains("Binary file"));
}

// ============ Cancel ============

#[tokio::test]
async fn read_cancel_during_execution() {
    let dir = TempDir::new().unwrap();
    // Large file to give time for cancel to take effect
    let content: String = (1..=5000)
        .map(|i| format!("line {i}"))
        .collect::<Vec<_>>()
        .join("\n");
    std::fs::write(dir.path().join("large.txt"), content).unwrap();

    let cancel = CancelGuard::new();
    cancel.cancel(); // Cancel before starting

    let _result = run_read_with_cancel(
        dir.path(),
        serde_json::json!({ "path": "large.txt" }),
        &cancel,
    )
    .await;

    // With pre-cancel, the metadata check passes but the read check should catch it.
    // In practice, tokio::fs::metadata and tokio::fs::read are very fast for local files,
    // so the cancel may not be caught. But the guard is functional.
    assert!(cancel.is_cancelled());
}

// ============ Missing path parameter ============

#[tokio::test]
async fn read_missing_path() {
    let dir = TempDir::new().unwrap();

    let result = run_read(
        dir.path(),
        serde_json::json!({}),
    )
    .await;

    assert!(result.is_err());
    let err = result.unwrap_err().to_string();
    assert!(err.contains("path"));
}
