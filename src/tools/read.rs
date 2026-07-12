//! Read tool — text file reading with offset/limit and binary detection.
//!
//! Reads files with configurable line offset and limit (max 2000 lines / 50 KB).
//! Detects binary files and returns an error instead of corrupting context.
//! Supports cooperative cancellation via `CancelGuard`.

use super::{CancelGuard, ToolDefinition, ToolHandler};
use async_trait::async_trait;
use std::path::{Path, PathBuf};
use tokio::sync::mpsc;

/// Maximum lines to return in a single read.
const MAX_LINES: usize = 2000;
/// Maximum bytes to return in a single read.
const MAX_BYTES: usize = 50 * 1024;

pub struct ReadTool;

impl std::fmt::Debug for ReadTool {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ReadTool")
    }
}

impl ToolDefinition for ReadTool {
    fn definition(&self) -> (String, String, serde_json::Value) {
        (
            "read".to_string(),
            "Read the contents of a file. Supports text files and images (jpg, png, gif, webp). \
             Images are sent as attachments. For text files, output is truncated to 2000 lines \
             or 50KB (whichever is hit first). Use offset/limit for large files. \
             When you need the full file, continue with offset until complete."
                .to_string(),
            serde_json::json!({
                "type": "object",
                "properties": {
                    "path": {"type": "string", "description": "Path to the file to read (relative or absolute)"},
                    "offset": {"type": "number", "description": "Line number to start reading from (1-indexed)"},
                    "limit": {"type": "number", "description": "Maximum number of lines to read"}
                },
                "required": ["path"],
                "additionalProperties": false
            }),
        )
    }
}

#[async_trait]
impl ToolHandler for ReadTool {
    async fn execute(
        &self,
        args: serde_json::Value,
        cwd: &Path,
        _output_tx: Option<&mpsc::Sender<String>>,
        cancel: &CancelGuard,
    ) -> anyhow::Result<String> {
        let path_str = args
            .get("path")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("Missing required parameter: path"))?;

        if path_str.is_empty() {
            return Err(anyhow::anyhow!("path cannot be empty"));
        }

        let path = if Path::new(&path_str).is_absolute() {
            PathBuf::from(&path_str)
        } else {
            cwd.join(path_str)
        };

        // Async existence check
        if let Err(e) = tokio::fs::metadata(&path).await {
            return Err(anyhow::anyhow!("File not found: {} — {}", path.display(), e));
        }
        if cancel.is_cancelled() {
            return Err(anyhow::anyhow!("Operation cancelled"));
        }

        // Parse offset (1-indexed) and limit
        let offset = args.get("offset").and_then(serde_json::Value::as_u64);
        let limit = args.get("limit").and_then(serde_json::Value::as_u64).map(|v| v as usize);

        // Async file read
        let bytes = tokio::fs::read(&path)
            .await
            .map_err(|e| anyhow::anyhow!("Failed to read file: {e}"))?;
        if cancel.is_cancelled() {
            return Err(anyhow::anyhow!("Operation cancelled"));
        }

        // Check if binary (null bytes in first 512 bytes)
        let is_binary = bytes.iter().take(512).any(|&b| b == 0);
        if is_binary {
            return Ok(format!(
                "Binary file ({} bytes). Cannot display contents.",
                bytes.len()
            ));
        }

        let content = String::from_utf8_lossy(&bytes).to_string();
        let all_lines: Vec<&str> = content.lines().collect();
        let total_lines = all_lines.len();

        // Convert 1-indexed offset to 0-indexed start
        let start = if let Some(o) = offset {
            let o = o as usize;
            if o > total_lines {
                return Err(anyhow::anyhow!(
                    "Offset {o} is beyond end of file ({total_lines} lines total)"
                ));
            }
            o - 1
        } else {
            0
        };

        // Determine effective limit
        let effective_limit = limit.unwrap_or(MAX_LINES);

        // Dual truncation: select lines while tracking both line count and byte count.
        // Never returns partial lines.
        let mut output_lines: Vec<String> = Vec::new();
        let mut output_bytes: usize = 0;
        let mut truncated_by: Option<&str> = None;
        let mut lines_selected: usize = 0;

        for line in all_lines.iter().skip(start) {
            if lines_selected >= effective_limit {
                truncated_by = Some("lines");
                break;
            }
            // Cost of this line: the line text + newline separator (except first line)
            let line_cost = line.len() + usize::from(!output_lines.is_empty());
            if output_bytes + line_cost > MAX_BYTES {
                truncated_by = Some("bytes");
                break;
            }
            output_lines.push(line.to_string());
            output_bytes += line_cost;
            lines_selected += 1;
        }

        // Check if first line alone exceeds byte limit
        if output_lines.is_empty() && start < total_lines {
            let first_line = all_lines[start];
            if first_line.len() > MAX_BYTES {
                let first_line_size = format_bytes(first_line.len());
                return Ok(format!(
                    "[Line {} is {}, exceeds {} limit. Use bash: sed -n '{}p' {} | head -c {}]",
                    start + 1,
                    first_line_size,
                    format_bytes(MAX_BYTES),
                    start + 1,
                    path_str,
                    MAX_BYTES,
                ));
            }
        }

        // Trim trailing empty lines
        while matches!(output_lines.last(), Some(l) if l.is_empty()) {
            output_lines.pop();
        }

        // Build output
        let end = start + lines_selected;
        let content_text = output_lines.join("\n");

        let mut output = String::new();

        // Header
        if offset.is_some() || limit.is_some() {
            output.push_str(&format!(
                "Showing lines {}-{} of {}:\n",
                start + 1,
                end,
                total_lines,
            ));
        } else {
            output.push_str(&format!("{total_lines} lines:\n"));
        }
        output.push_str(&content_text);

        // Continuation hint
        let remaining = total_lines - end;
        if remaining > 0 {
            let next_offset = end + 1;
            if truncated_by == Some("lines") {
                output.push_str(&format!(
                    "\n\n[Showing lines {}-{} of {} ({} line limit). Use offset={} to continue.]",
                    start + 1, end, total_lines, MAX_LINES, next_offset,
                ));
            } else if truncated_by == Some("bytes") {
                output.push_str(&format!(
                    "\n\n[Showing lines {}-{} of {} ({} limit). Use offset={} to continue.]",
                    start + 1, end, total_lines, format_bytes(MAX_BYTES), next_offset,
                ));
            } else {
                output.push_str(&format!(
                    "\n\n[{remaining} more lines in file. Use offset={next_offset} to continue.]",
                ));
            }
        }

        Ok(output)
    }
}

fn format_bytes(bytes: usize) -> String {
    if bytes < 1024 {
        format!("{bytes}B")
    } else if bytes < 1024 * 1024 {
        format!("{:.1}KB", bytes as f64 / 1024.0)
    } else {
        format!("{:.1}MB", bytes as f64 / (1024.0 * 1024.0))
    }
}
