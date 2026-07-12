//! Write tool — file creation with parent directory auto-creation.
//!
//! Writes content to files, creating parent directories as needed. Uses
//! per-file serialization queues to prevent concurrent write conflicts.
//! Supports cooperative cancellation via `CancelGuard`.

use super::{CancelGuard, ToolDefinition, ToolHandler};
use async_trait::async_trait;
use std::path::{Path, PathBuf};
use tokio::sync::mpsc;

pub struct WriteTool;

impl std::fmt::Debug for WriteTool {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("WriteTool")
    }
}

impl ToolDefinition for WriteTool {
    fn definition(&self) -> (String, String, serde_json::Value) {
        (
            "write".to_string(),
            "Write content to a file. Creates the file if it doesn't exist, overwrites if it does. \
             Automatically creates parent directories."
                .to_string(),
            serde_json::json!({
                "type": "object",
                "properties": {
                    "path": {"type": "string", "description": "Path to the file to write (relative or absolute)"},
                    "content": {"type": "string", "description": "Content to write to the file"}
                },
                "required": ["path", "content"],
                "additionalProperties": false
            }),
        )
    }
}

#[async_trait]
impl ToolHandler for WriteTool {
    async fn execute(
        &self,
        args: serde_json::Value,
        cwd: &Path,
        _output_tx: Option<&mpsc::Sender<String>>,
        cancel: &CancelGuard,
    ) -> anyhow::Result<String> {
        let path_str = match args.get("path").and_then(|v| v.as_str()) {
            Some(s) => s.to_string(),
            None => {
                return Err(anyhow::anyhow!("Missing required parameter: path"));
            }
        };

        let content = match args.get("content").and_then(|v| v.as_str()) {
            Some(s) => s.to_string(),
            None => {
                return Err(anyhow::anyhow!("Missing required parameter: content"));
            }
        };

        let path = if Path::new(&path_str).is_absolute() {
            PathBuf::from(&path_str)
        } else {
            cwd.join(&path_str)
        };

        // Create parent directories (async)
        if let Some(parent) = path.parent()
            && let Err(e) = tokio::fs::create_dir_all(parent).await
        {
            return Err(anyhow::anyhow!("Failed to create directories: {e}"));
        }
        if cancel.is_cancelled() {
            return Err(anyhow::anyhow!("Operation cancelled"));
        }

        let size = content.len();

        // Write the file with per-file serialization (async)
        super::file_queue::with_file_queue(&path, async {
            tokio::fs::write(&path, &content).await
        })
        .await??;

        Ok(format!("Successfully wrote {size} bytes to {path_str}"))
    }
}
