//! Tests for hook execution.

use std::path::PathBuf;

use super::executor::{MAX_OUTPUT_BYTES, process_output};
use super::{HookContext, execute_hook};
use crate::hooks::loader::{Hook, HookEvent, HookSource};

#[tokio::test]
async fn execute_hook_success() {
    let hook = Hook {
        event: HookEvent::PreToolUse,
        name: "test.sh".to_string(),
        command: "echo 'hello world'".to_string(),
        source: HookSource::Global,
        skill_name: None,
        skill_dir: None,
        matcher: super::loader::ToolMatcher::All,
    };
    let context = HookContext {
        event: HookEvent::PreToolUse,
        hook_name: "test.sh".to_string(),
        cwd: PathBuf::from("."),
        tool_name: Some("Bash".to_string()),
        tool_input: Some(serde_json::json!({"command": "ls"})),
        tool_result: None,
        prompt: None,
        compact_trigger: None,
        compact_messages_removed: None,
    };

    let result = execute_hook(&hook, &context).await;
    assert!(result.success());
    assert!(result.stdout.contains("hello world"));
    assert_eq!(result.stderr, "");
}

#[tokio::test]
async fn execute_hook_blocking() {
    let hook = Hook {
        event: HookEvent::PreToolUse,
        name: "block.sh".to_string(),
        command: "echo 'blocked' >&2; exit 2".to_string(),
        source: HookSource::Project,
        skill_name: None,
        skill_dir: None,
        matcher: super::loader::ToolMatcher::All,
    };
    let context = HookContext {
        event: HookEvent::PreToolUse,
        hook_name: "block.sh".to_string(),
        cwd: PathBuf::from("."),
        tool_name: Some("Bash".to_string()),
        tool_input: None,
        tool_result: None,
        prompt: None,
        compact_trigger: None,
        compact_messages_removed: None,
    };

    let result = execute_hook(&hook, &context).await;
    assert!(result.blocked());
    assert!(result.stderr.contains("blocked"));
}

#[tokio::test]
async fn execute_hook_non_blocking_error() {
    let hook = Hook {
        event: HookEvent::PostToolUse,
        name: "warn.sh".to_string(),
        command: "echo 'warning' >&2; exit 1".to_string(),
        source: HookSource::Global,
        skill_name: None,
        skill_dir: None,
        matcher: super::loader::ToolMatcher::All,
    };
    let context = HookContext {
        event: HookEvent::PostToolUse,
        hook_name: "warn.sh".to_string(),
        cwd: PathBuf::from("."),
        tool_name: Some("Write".to_string()),
        tool_input: None,
        tool_result: None,
        prompt: None,
        compact_trigger: None,
        compact_messages_removed: None,
    };

    let result = execute_hook(&hook, &context).await;
    assert!(!result.success());
    assert!(!result.blocked());
    assert!(result.stderr.contains("warning"));
}

#[tokio::test]
async fn execute_hook_environment_variables() {
    let hook = Hook {
        event: HookEvent::UserPromptSubmit,
        name: "env-test.sh".to_string(),
        command: "echo HOOK_EVENT=$HOOK_EVENT HOOK_NAME=$HOOK_NAME PROMPT=$PROMPT".to_string(),
        source: HookSource::Global,
        skill_name: None,
        skill_dir: None,
        matcher: super::loader::ToolMatcher::All,
    };
    let context = HookContext {
        event: HookEvent::UserPromptSubmit,
        hook_name: "env-test.sh".to_string(),
        cwd: PathBuf::from("."),
        tool_name: None,
        tool_input: None,
        tool_result: None,
        prompt: Some("test prompt".to_string()),
        compact_trigger: None,
        compact_messages_removed: None,
    };

    let result = execute_hook(&hook, &context).await;
    assert!(result.stdout.contains("HOOK_EVENT=UserPromptSubmit"));
    assert!(result.stdout.contains("HOOK_NAME=env-test.sh"));
    assert!(result.stdout.contains("PROMPT=test prompt"));
}

#[tokio::test]
async fn execute_hook_timeout() {
    let hook = Hook {
        event: HookEvent::PreToolUse,
        name: "slow.sh".to_string(),
        command: "sleep 31".to_string(),
        source: HookSource::Global,
        skill_name: None,
        skill_dir: None,
        matcher: super::loader::ToolMatcher::All,
    };
    let context = HookContext {
        event: HookEvent::PreToolUse,
        hook_name: "slow.sh".to_string(),
        cwd: PathBuf::from("."),
        tool_name: None,
        tool_input: None,
        tool_result: None,
        prompt: None,
        compact_trigger: None,
        compact_messages_removed: None,
    };

    let result = execute_hook(&hook, &context).await;
    assert!(!result.success());
    assert!(result.stderr.contains("timed out"));
}

#[tokio::test]
async fn execute_hook_output_truncation() {
    // Generate a large output
    let large_text = "x".repeat(15_000);
    let hook = Hook {
        event: HookEvent::Stop,
        name: "big-output.sh".to_string(),
        command: format!("echo '{large_text}'"),
        source: HookSource::Global,
        skill_name: None,
        skill_dir: None,
        matcher: super::loader::ToolMatcher::All,
    };
    let context = HookContext {
        event: HookEvent::Stop,
        hook_name: "big-output.sh".to_string(),
        cwd: PathBuf::from("."),
        tool_name: None,
        tool_input: None,
        tool_result: None,
        prompt: None,
        compact_trigger: None,
        compact_messages_removed: None,
    };

    let result = execute_hook(&hook, &context).await;
    assert!(result.stdout.len() <= MAX_OUTPUT_BYTES + 100); // +100 for truncation note
    assert!(result.stdout.contains("truncated"));
}

#[tokio::test]
async fn execute_hook_skill_env_vars() {
    let hook = Hook {
        event: HookEvent::PreToolUse,
        name: "planning:inject.sh".to_string(),
        command: "echo SKILL_NAME=$SKILL_NAME SKILL_DIR=$SKILL_DIR".to_string(),
        source: HookSource::Global,
        skill_name: Some("planning".to_string()),
        skill_dir: Some(PathBuf::from("/skills/planning")),
        matcher: super::loader::ToolMatcher::All,
    };
    let context = HookContext {
        event: HookEvent::PreToolUse,
        hook_name: "planning:inject.sh".to_string(),
        cwd: PathBuf::from("."),
        tool_name: Some("Write".to_string()),
        tool_input: None,
        tool_result: None,
        prompt: None,
        compact_trigger: None,
        compact_messages_removed: None,
    };

    let result = execute_hook(&hook, &context).await;
    assert!(result.stdout.contains("SKILL_NAME=planning"));
    assert!(result.stdout.contains("SKILL_DIR=/skills/planning"));
}

#[tokio::test]
async fn execute_hook_no_skill_env_vars_for_directory_hook() {
    let hook = Hook {
        event: HookEvent::PreToolUse,
        name: "dir.sh".to_string(),
        command: "echo NAME=${SKILL_NAME:-unset} DIR=${SKILL_DIR:-unset}".to_string(),
        source: HookSource::Global,
        skill_name: None,
        skill_dir: None,
        matcher: super::loader::ToolMatcher::All,
    };
    let context = HookContext {
        event: HookEvent::PreToolUse,
        hook_name: "dir.sh".to_string(),
        cwd: PathBuf::from("."),
        tool_name: None,
        tool_input: None,
        tool_result: None,
        prompt: None,
        compact_trigger: None,
        compact_messages_removed: None,
    };

    let result = execute_hook(&hook, &context).await;
    assert!(result.stdout.contains("NAME=unset"));
    assert!(result.stdout.contains("DIR=unset"));
}

#[test]
fn process_output_multibyte_at_boundary_no_panic() {
    // Build a string where byte MAX_OUTPUT_BYTES falls inside a 3-byte UTF-8 char.
    // "☀" (U+2600) encodes as [0xE2, 0x98, 0x80] — 3 bytes.
    // Pad with ASCII to position the 3-byte char so byte MAX_OUTPUT_BYTES hits its middle.
    let padding = "x".repeat(MAX_OUTPUT_BYTES - 1); // 9999 bytes
    let text = format!("{padding}☀");
    assert_eq!(text.len(), MAX_OUTPUT_BYTES + 2); // 10002 bytes total
    // Byte layout: 0..9998 = 'x' (9999 chars), 9999 = 0xE2, 10000 = 0x98, 10001 = 0x80
    // MAX_OUTPUT_BYTES (10000) falls on the 2nd byte of ☀ — a continuation byte.

    // This must NOT panic — truncate_to_bytes scans backward to find valid boundary.
    let result = process_output(text.as_bytes());
    // Output should be truncated to boundary before the multi-byte char.
    assert!(result.len() <= MAX_OUTPUT_BYTES + 100); // +100 for truncation note
    assert!(result.contains("truncated"));
}
