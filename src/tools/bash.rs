//! Bash tool — shell command execution with timeout and output capture.
//!
//! Spawns shell commands via `tokio::process::Command`, captures stdout/stderr
//! with streaming output, and sanitizes ANSI escape sequences and control
//! characters before returning results to the LLM context. Supports cooperative
//! cancellation via `CancelGuard`.

use super::{CancelGuard, ToolDefinition, ToolHandler};
use async_trait::async_trait;
use std::path::Path;
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncRead};
use tokio::sync::mpsc;

pub struct BashTool;

impl std::fmt::Debug for BashTool {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("BashTool")
    }
}

impl ToolDefinition for BashTool {
    fn definition(&self) -> (String, String, serde_json::Value) {
        (
            "bash".to_string(),
            "Execute a shell command. Output is captured and returned.".to_string(),
            serde_json::json!({
                "type": "object",
                "properties": {
                    "command": {"type": "string", "description": "Shell command to execute"},
                    "timeout": {"type": "number", "description": "Timeout in seconds (default: 30)"}
                },
                "required": ["command"],
                "additionalProperties": false
            }),
        )
    }
}

/// Maximum output size before truncation.
const MAX_OUTPUT_BYTES: usize = 50 * 1024;

#[async_trait]
impl ToolHandler for BashTool {
    async fn execute(
        &self,
        args: serde_json::Value,
        cwd: &Path,
        output_tx: Option<&mpsc::Sender<String>>,
        cancel: &CancelGuard,
    ) -> anyhow::Result<String> {
        let command = match args.get("command").and_then(|v| v.as_str()) {
            Some(s) => s.to_string(),
            None => {
                return Err(anyhow::anyhow!("Missing required parameter: command"));
            }
        };

        let timeout_secs = args
            .get("timeout")
            .and_then(serde_json::Value::as_f64)
            .unwrap_or(30.0) as u64;

        if timeout_secs == 0 {
            return Err(anyhow::anyhow!("timeout must be greater than 0"));
        }

        let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/bash".to_string());

        // Spawn the process with piped stdout/stderr
        let mut child = match tokio::process::Command::new(&shell)
            .arg("-c")
            .arg(&command)
            .current_dir(cwd)
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
        {
            Ok(child) => child,
            Err(e) => {
                return Err(anyhow::anyhow!("Failed to execute command: {e}"));
            }
        };

        // Check cancel after spawn
        if cancel.is_cancelled() {
            let _ = child.kill().await;
            return Err(anyhow::anyhow!("Operation cancelled"));
        }

        let stdout = child.stdout.take().expect("stdout is piped");
        let stderr = child.stderr.take().expect("stderr is piped");

        // Clone cancel guard for stream readers
        let cancel_stdout = cancel.clone();
        let cancel_stderr = cancel.clone();

        // Read stdout and stderr concurrently into separate buffers
        let stdout_fut = read_stream(stdout, output_tx, &cancel_stdout);
        let stderr_fut = read_stream(stderr, output_tx, &cancel_stderr);

        // Wait for process exit with timeout (runs concurrently with stream reads)
        let wait_fut = async {
            tokio::time::timeout(Duration::from_secs(timeout_secs), child.wait()).await
        };

        // Poll cancellation in a loop — fires when CancelGuard is set
        let cancel_fut = async {
            loop {
                if cancel.is_cancelled() {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
        };

        // Race: streams+wait complete, or user cancels
        let result = tokio::select! {
            result = async { tokio::join!(stdout_fut, stderr_fut, wait_fut) } => Some(result),
            () = cancel_fut => None,
        };

        if let Some(((stdout_output, _), (stderr_output, _), exit_result)) = result {
            let combined = format!("{stdout_output}{stderr_output}");

            let exit_code = match exit_result {
                Ok(Ok(status)) => status.code().unwrap_or(-1),
                Ok(Err(_)) => -1,
                Err(_) => {
                    // Timed out — kill child and return collected output
                    let _ = child.kill().await;
                    let sanitized = sanitize_output(&combined);
                    let output_text = if sanitized.len() > MAX_OUTPUT_BYTES {
                        let truncated = truncate_to_bytes(&sanitized, MAX_OUTPUT_BYTES);
                        format!(
                            "{}\n\n... (output truncated at 50KB, {} bytes total)\nCommand timed out after {} seconds",
                            truncated,
                            sanitized.len(),
                            timeout_secs,
                        )
                    } else {
                        format!(
                            "{}\nCommand timed out after {} seconds",
                            sanitized.trim_end(),
                            timeout_secs,
                        )
                    };
                    return Err(anyhow::anyhow!(output_text));
                }
            };

            // Sanitize output: strip ANSI escape codes and control characters.
            // Without this, raw terminal output (colored ls, progress bars, etc.)
            // injects ESC bytes and control chars into the LLM context, which can
            // corrupt the chat template and cause the model to return empty responses.
            // Modeled on pi-coding-agent's stripAnsi() + sanitizeBinaryOutput().
            let sanitized = sanitize_output(&combined);

            let output_text = if sanitized.len() > MAX_OUTPUT_BYTES {
                let truncated = truncate_to_bytes(&sanitized, MAX_OUTPUT_BYTES);
                format!(
                    "{}\n\n... (output truncated at 50KB, {} bytes total)\nExit code: {}",
                    truncated,
                    sanitized.len(),
                    exit_code,
                )
            } else {
                format!("{}\nExit code: {}", sanitized.trim_end(), exit_code)
            };

            Ok(output_text)
        } else {
            // Cancelled — kill child, discard partial output
            let _ = child.kill().await;
            Err(anyhow::anyhow!("Operation cancelled"))
        }
    }
}

/// Sanitize bash output for inclusion in LLM context.
///
/// Two-stage pipeline modeled on pi-coding-agent's `stripAnsi()` + `sanitizeBinaryOutput()`:
/// 1. **ANSI stripping** — removes CSI, OSC, and other escape sequences. Handles both
///    7-bit ESC (0x1B) and 8-bit C1 introducers (0x9B CSI, 0x9D OSC, 0x9C ST).
/// 2. **Binary sanitization** — removes remaining control characters and Unicode format
///    characters (0xFFF9–0xFFFB) that can crash string-width or confuse tokenizers.
pub(crate) fn sanitize_output(input: &str) -> String {
    let stripped = strip_ansi(input);
    sanitize_binary(&stripped)
}

/// Strip ANSI escape sequences from terminal output.
///
/// Handles both 7-bit (ESC = 0x1B) and 8-bit C1 control introducers:
/// - **CSI** (0x1B[ or 0x9B) — cursor movement, colors, attributes
/// - **OSC** (0x1B] or 0x9D) — window title, icons, etc.
/// - **ST** terminator: BEL (0x07), ESC\\ (0x1B\\), or 0x9C
///
/// Modeled on pi-coding-agent's `stripAnsi()` (chalk/strip-ansi regex).
pub(crate) fn strip_ansi(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut iter = input.chars().peekable();

    while let Some(ch) = iter.next() {
        match ch {
            // 8-bit C1 CSI (0x9B) — single-byte CSI introducer (replaces ESC[)
            '\u{9B}' => {
                // Already the complete introducer — skip params + final byte
                loop {
                    match iter.next() {
                        Some(c) if is_ansi_final_byte(c) => break,
                        Some(_) => continue,
                        None => break,
                    }
                }
            }
            // 8-bit C1 OSC (0x9D) — single-byte OSC introducer (replaces ESC])
            '\u{9D}' => {
                // Already the complete introducer — skip until terminator
                loop {
                    match iter.next() {
                        Some('\x07') => break,       // BEL terminator
                        Some('\x1b') => {
                            if iter.peek() == Some(&'\\') {
                                iter.next(); // ESC\\ ST terminator
                            }
                            break;
                        }
                        Some('\u{9C}') => break,    // C1 ST terminator
                        Some(_) => continue,
                        None => break,
                    }
                }
            }
            // 7-bit ESC (0x1B) — multi-byte ANSI sequence introducer
            '\x1b' => {
                match iter.peek() {
                    Some(&']') => {
                        // OSC sequence: ESC]
                        iter.next(); // consume ']'
                        loop {
                            match iter.next() {
                                Some('\x07') => break,       // BEL terminator
                                Some('\x1b') => {
                                    if iter.peek() == Some(&'\\') {
                                        iter.next(); // ESC\\ ST terminator
                                    }
                                    break;
                                }
                                Some('\u{9C}') => break,    // C1 ST terminator
                                Some(_) => continue,
                                None => break,
                            }
                        }
                    }
                    Some('[' | '(' | ')' | '#' | '?' | '=' | '^') => {
                        // CSI or similar: ESC[ ... final byte
                        iter.next(); // consume introducer
                        loop {
                            match iter.next() {
                                Some(c) if is_ansi_final_byte(c) => break,
                                Some(_) => continue,
                                None => break,
                            }
                        }
                    }
                    _ => {
                        // Unknown ESC sequence — skip next char if it looks like an introducer
                        // to prevent orphaned fragments like `[31m` in output
                        if let Some(&next) = iter.peek() {
                            matches!(next, '[' | ']' | '(' | ')' | '#' | '?' | '=' | '^')
                                .then(|| iter.next());
                        }
                    }
                }
            }
            _ => out.push(ch),
        }
    }

    out
}

/// Sanitize binary/problematic characters from text.
///
/// Removes characters that can crash string-width libraries or confuse LLM tokenizers:
/// - Control characters (0x00–0x1F except `\t`, `\n`, `\r`) and DEL (0x7F)
/// - C1 control characters (0x80–0x9F) — includes 0x9B CSI, 0x9D OSC, 0x9C ST
/// - Unicode format characters (0xFFF9–0xFFFB)
///
/// Modeled on pi-coding-agent's `sanitizeBinaryOutput()`.
pub(crate) fn sanitize_binary(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    for ch in input.chars() {
        let code = ch as u32;
        match code {
            // Allow tab, newline, carriage return
            0x09 | 0x0A | 0x0D => out.push(ch),
            // Drop control characters (C0 + DEL + C1)
            0x00..=0x1F | 0x7F | 0x80..=0x9F => {}
            // Drop Unicode format characters
            0xFFF9..=0xFFFB => {}
            _ => out.push(ch),
        }
    }
    out
}

/// Check if a character is a valid ANSI C0 final byte.
/// Final bytes end a CSI or similar sequence (e.g., the 'm' in \x1b[31m).
pub(crate) fn is_ansi_final_byte(c: char) -> bool {
    matches!(c, 'A'..='Z' | 'a'..='z' | '~' | '=' | '>' | '!' | '/')
}

/// Truncate a string to a maximum byte length, respecting UTF-8 character boundaries.
///
/// Returns the longest prefix of `input` whose UTF-8 encoding is at most `max_bytes`.
/// Never splits a multi-byte character — if the next character would exceed the limit,
/// it is dropped entirely.
pub(crate) fn truncate_to_bytes(input: &str, max_bytes: usize) -> &str {
    if input.len() <= max_bytes {
        return input;
    }
    // Try to slice at the exact byte limit. If it falls in the middle of a
    // multi-byte character, scan backward to find the last valid UTF-8 boundary.
    if let Some(prefix) = input.get(..max_bytes) {
        prefix
    } else {
        // max_bytes is in the middle of a multi-byte character.
        // bytes[max_bytes] is a continuation byte. Scan backward to find
        // the start of that character (first non-continuation byte).
        let bytes = input.as_bytes();
        let mut boundary = max_bytes;
        while boundary > 0 && (bytes[boundary] & 0xC0) == 0x80 {
            boundary -= 1;
        }
        // boundary is now at a valid character start (or 0)
        input.get(..boundary).unwrap_or("")
    }
}

/// Read a stream until EOF or cancellation.
/// Optionally sends each line via a channel.
/// Returns (`full_output`, `total_bytes`).
async fn read_stream(
    stream: impl AsyncRead + Unpin,
    tx: Option<&mpsc::Sender<String>>,
    cancel: &CancelGuard,
) -> (String, usize) {
    let mut output = String::new();
    let mut reader = tokio::io::BufReader::new(stream);
    let mut buf = Vec::new();

    loop {
        if cancel.is_cancelled() {
            break;
        }
        buf.clear();
        match reader.read_until(b'\n', &mut buf).await {
            Ok(0) => break, // EOF
            Ok(_) => {
                let text = String::from_utf8_lossy(&buf).to_string();
                if let Some(tx) = tx {
                    let _ = tx.send(text.clone()).await;
                }
                output.push_str(&text);
            }
            Err(_) => break,
        }
    }

    let len = output.len();
    (output, len)
}
