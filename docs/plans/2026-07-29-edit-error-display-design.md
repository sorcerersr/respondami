# Edit Tool Call — Inline Error Message Display

**Date:** 2026-07-29

## Problem

When an `edit` tool call fails (e.g., `oldText` not found in file), the LLM receives the error message as tool result content. However, the TUI discards this error string and only displays `"Failed to edit {path}"` — no details about why it failed.

## Solution

Store the error message in `EditToolCall` and display it inline beneath the failure summary line.

## Changes

### 1. `src/tui/messages/tool_call/edit.rs`

**Add `error_message` field to `EditToolCall`:**

```rust
pub struct EditToolCall {
    pub path: String,
    pub edits: Vec<EditDiff>,
    pub has_error: bool,
    pub error_message: Option<String>,  // NEW
    pub expanded: bool,
}
```

**Capture result string in `from_args()`:**

Change `_result` parameter to `result`, store it when `has_error` is true.

**Include error in `content_lines()`:**

Append `"⚠ {error_message}"` line after the failure summary.

### 2. No other files affected

- `build_content_lines()` in `mod.rs` already styles all content red when `has_error` is true
- No theme changes needed
- No changes to agent loop, session persistence, or provider layer

## Example Output

```
edit: src/tools/edit.rs
  Failed to edit src/tools/edit.rs
  ⚠ Matching oldText not found in file
```
