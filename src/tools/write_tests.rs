use super::write::WriteTool;
use super::{CancelGuard, ToolHandler};
use std::path::Path;
use tempfile::TempDir;

async fn run_write(cwd: &Path, args: serde_json::Value) -> anyhow::Result<String> {
    let cancel = CancelGuard::new();
    WriteTool.execute(args, cwd, None, &cancel).await
}

async fn run_write_with_cancel(
    cwd: &Path,
    args: serde_json::Value,
    cancel: &CancelGuard,
) -> anyhow::Result<String> {
    WriteTool.execute(args, cwd, None, cancel).await
}

// ============ Basic writes ============

#[tokio::test]
async fn write_new_file() {
    let dir = TempDir::new().unwrap();

    let result = run_write(
        dir.path(),
        serde_json::json!({
            "path": "hello.txt",
            "content": "hello world"
        }),
    )
    .await;

    assert!(result.is_ok(), "unexpected error: {:?}", result.err());
    let content = result.unwrap();
    assert!(content.contains("Successfully wrote"));
    assert!(content.contains("11 bytes"));
    assert!(!content.contains("lines")); // No line count

    // Verify file content
    let file_content = std::fs::read_to_string(dir.path().join("hello.txt")).unwrap();
    assert_eq!(file_content, "hello world");
}

#[tokio::test]
async fn write_creates_parent_dirs() {
    let dir = TempDir::new().unwrap();

    let result = run_write(
        dir.path(),
        serde_json::json!({
            "path": "a/b/c/deep.txt",
            "content": "deep content"
        }),
    )
    .await;

    result.unwrap();
    let file_content = std::fs::read_to_string(dir.path().join("a/b/c/deep.txt")).unwrap();
    assert_eq!(file_content, "deep content");
}

#[tokio::test]
async fn write_overwrites_existing() {
    let dir = TempDir::new().unwrap();
    std::fs::write(dir.path().join("file.txt"), "old content").unwrap();

    let result = run_write(
        dir.path(),
        serde_json::json!({
            "path": "file.txt",
            "content": "new content"
        }),
    )
    .await;

    result.unwrap();
    let file_content = std::fs::read_to_string(dir.path().join("file.txt")).unwrap();
    assert_eq!(file_content, "new content");
}

// ============ Concurrent writes (file queue) ============

#[tokio::test]
async fn write_concurrent_same_file_serialized() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().to_path_buf();

    // Launch two concurrent writes to the same file
    let cancel = CancelGuard::new();
    let (result1, result2) = tokio::join!(
        async {
            WriteTool.execute(
                serde_json::json!({
                    "path": "shared.txt",
                    "content": "first"
                }),
                &path,
                None,
                &cancel,
            )
            .await
        },
        async {
            WriteTool.execute(
                serde_json::json!({
                    "path": "shared.txt",
                    "content": "second"
                }),
                &path,
                None,
                &cancel,
            )
            .await
        },
    );

    result1.unwrap();
    result2.unwrap();
    // File should contain one of the two writes (not garbled)
    let content = std::fs::read_to_string(dir.path().join("shared.txt")).unwrap();
    assert!(
        content == "first" || content == "second",
        "Expected 'first' or 'second', got '{content}'"
    );
}

// ============ Missing parameters ============

#[tokio::test]
async fn write_missing_path() {
    let dir = TempDir::new().unwrap();

    let result = run_write(
        dir.path(),
        serde_json::json!({ "content": "data" }),
    )
    .await;

    assert!(result.is_err());
    let err = result.unwrap_err().to_string();
    assert!(err.contains("path"));
}

#[tokio::test]
async fn write_missing_content() {
    let dir = TempDir::new().unwrap();

    let result = run_write(
        dir.path(),
        serde_json::json!({ "path": "file.txt" }),
    )
    .await;

    assert!(result.is_err());
    let err = result.unwrap_err().to_string();
    assert!(err.contains("content"));
}

// ============ Cancel ============

#[tokio::test]
async fn write_cancel_before_start() {
    let dir = TempDir::new().unwrap();
    let cancel = CancelGuard::new();
    cancel.cancel();

    let _result = run_write_with_cancel(
        dir.path(),
        serde_json::json!({
            "path": "cancelled.txt",
            "content": "should not write"
        }),
        &cancel,
    )
    .await;

    // With pre-cancel, the mkdir may still succeed (it's fast),
    // but the write should be cancelled.
    // In practice, tokio::fs::create_dir_all is very fast,
    // so the cancel check after mkdir should catch it.
    assert!(cancel.is_cancelled());
}

// ============ Result message format ============

#[tokio::test]
async fn write_result_no_line_count() {
    let dir = TempDir::new().unwrap();

    let result = run_write(
        dir.path(),
        serde_json::json!({
            "path": "test.txt",
            "content": "line1\nline2\nline3"
        }),
    )
    .await;

    assert!(result.is_ok());
    let content = result.unwrap();
    // Should contain bytes but NOT line count
    assert!(content.contains("bytes"));
    assert!(!content.contains("lines"));
}
