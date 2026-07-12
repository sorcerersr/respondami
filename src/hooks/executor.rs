//! Hook execution with environment variables, timeout, and output capture.

use std::collections::HashMap;
use std::path::PathBuf;
use std::process::Stdio;
use std::time::Duration;

use tokio::process::Command;

use crate::tools::truncate_to_bytes;

use super::Hook;

/// Maximum output size before falling back to a temp file.
pub(crate) const MAX_OUTPUT_BYTES: usize = 10_000;

/// Default timeout for hook execution (30 seconds, matching Claude).
const HOOK_TIMEOUT_SECS: u64 = 30;

/// Context passed to a hook script as environment variables.
#[derive(Debug, Clone)]
pub struct HookContext {
    /// The hook event that triggered this execution.
    pub event: super::HookEvent,
    /// The hook name (script filename).
    pub hook_name: String,
    /// Current working directory.
    pub cwd: PathBuf,
    /// Tool name (`PreToolUse`, `PostToolUse` only).
    pub tool_name: Option<String>,
    /// Tool input as JSON (`PreToolUse`, `PostToolUse` only).
    pub tool_input: Option<serde_json::Value>,
    /// Tool result as JSON (`PostToolUse` only).
    pub tool_result: Option<serde_json::Value>,
    /// User's prompt text (`UserPromptSubmit` only).
    pub prompt: Option<String>,
}

/// Result of executing a hook.
#[derive(Debug, Clone)]
pub struct HookResult {
    /// Exit code from the hook script.
    pub exit_code: i32,
    /// Standard output (capped at `MAX_OUTPUT_BYTES`).
    pub stdout: String,
    /// Standard error (capped at `MAX_OUTPUT_BYTES`).
    pub stderr: String,
    /// If stdout exceeded `MAX_OUTPUT_BYTES`, this is the path to the temp file.
    pub stdout_file: Option<PathBuf>,
    /// If stderr exceeded `MAX_OUTPUT_BYTES`, this is the path to the temp file.
    pub stderr_file: Option<PathBuf>,
}

impl HookResult {
    /// Returns `true` if the hook was successful (exit 0).
    #[must_use]
    pub fn success(&self) -> bool {
        self.exit_code == 0
    }

    /// Returns `true` if the hook blocked the action (exit 2).
    #[must_use]
    pub fn blocked(&self) -> bool {
        self.exit_code == 2
    }
}

/// Execute a hook script with the given context.
///
/// Sets environment variables from `HookContext`, spawns the shell command,
/// captures stdout/stderr, and returns a `HookResult`.
///
/// If the hook script is not found, returns a `HookResult` with exit code 1
/// and a descriptive error in stderr.
pub async fn execute_hook(hook: &Hook, context: &HookContext) -> HookResult {
    // Build environment variables
    let mut env_vars = HashMap::new();
    env_vars.insert("HOOK_EVENT".to_string(), format!("{:?}", hook.event));
    env_vars.insert("HOOK_NAME".to_string(), hook.name.clone());
    env_vars.insert("CWD".to_string(), context.cwd.to_string_lossy().to_string());

    if let Some(tool_name) = &context.tool_name {
        env_vars.insert("TOOL_NAME".to_string(), tool_name.clone());
    }
    if let Some(tool_input) = &context.tool_input {
        env_vars.insert("TOOL_INPUT".to_string(), tool_input.to_string());
    }
    if let Some(tool_result) = &context.tool_result {
        env_vars.insert("TOOL_RESULT".to_string(), tool_result.to_string());
    }
    if let Some(prompt) = &context.prompt {
        env_vars.insert("PROMPT".to_string(), prompt.clone());
    }

    // Spawn the shell command
    let mut cmd = Command::new("bash");
    cmd.kill_on_drop(true)
        .arg("-c")
        .arg(&hook.command)
        .current_dir(&context.cwd)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    // Set environment variables
    for (key, value) in env_vars {
        cmd.env(key, value);
    }

    // Execute with timeout
    // `kill_on_drop(true)` ensures the child is killed if the timeout fires —
    // `wait_with_output()` takes ownership of `self`, so on timeout the future
    // is dropped and the child is reaped automatically.
    let child = match cmd.spawn() {
        Ok(child) => child,
        Err(e) => {
            return HookResult {
                exit_code: 1,
                stdout: String::new(),
                stderr: format!("Failed to spawn hook: {e}"),
                stdout_file: None,
                stderr_file: None,
            };
        }
    };

    let output = tokio::time::timeout(
        Duration::from_secs(HOOK_TIMEOUT_SECS),
        child.wait_with_output(),
    )
    .await;

    let output = match output {
        Ok(Ok(output)) => output,
        Ok(Err(e)) => {
            return HookResult {
                exit_code: 1,
                stdout: String::new(),
                stderr: format!("Hook execution error: {e}"),
                stdout_file: None,
                stderr_file: None,
            };
        }
        Err(_) => {
            // Timeout — child is killed by kill_on_drop when the future is dropped.
            return HookResult {
                exit_code: 1,
                stdout: String::new(),
                stderr: format!(
                    "Hook '{}' timed out after {} seconds",
                    hook.name, HOOK_TIMEOUT_SECS
                ),
                stdout_file: None,
                stderr_file: None,
            };
        }
    };

    // Process output with size cap
    let stdout = process_output(&output.stdout);
    let stderr = process_output(&output.stderr);

    HookResult {
        exit_code: output.status.code().unwrap_or(-1),
        stdout,
        stderr,
        stdout_file: None,
        stderr_file: None,
    }
}

/// Process raw output bytes, capping at `MAX_OUTPUT_BYTES`.
/// Returns the capped string (no temp file for now — that's a future enhancement).
pub(crate) fn process_output(raw: &[u8]) -> String {
    let text = String::from_utf8_lossy(raw).to_string();
    if text.len() <= MAX_OUTPUT_BYTES {
        text
    } else {
        // Truncate and add a note
        let truncated = truncate_to_bytes(&text, MAX_OUTPUT_BYTES);
        format!(
            "{}\n\n[Output truncated: {} characters total]",
            truncated,
            text.len()
        )
    }
}
