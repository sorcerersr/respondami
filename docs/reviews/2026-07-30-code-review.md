# Code Health Check — 2026-07-30

**Scope**: 160 Rust files, ~31,486 LOC, 3 crates (respondami, ratatui-widgets, ratatui-md)
**Focus**: Correctness bugs, code quality issues, dependency audit
**Methodology**: Three-pass review (correctness → quality → dependencies), dependency-ordered file traversal

## Summary

| Severity | Count |
|----------|-------|
| Critical | 1 |
| High | 4 |
| Medium | 5 |
| Low | 4 |

---

## Critical Findings

### C-1 Token usage double-counting causes inflated session statistics
- **Location**: `src/tui/app.rs:380-394` — `accumulate_token_usage()` (lines ~378-395)
- **Issue**: The function adds `delta.input_tokens` (which equals `usage.prompt_tokens`) to `cumulative_usage.input_tokens` on every Usage event. Since `prompt_tokens` represents the full context window size for that request (e.g., 32K tokens), and each turn's context grows, this accumulates to massive inflated totals. After 10 turns with ~30K prompt tokens each, the cumulative shows ~300K input tokens when actual session input was far less.
- **Why it matters**: Token statistics displayed in the status bar and token stats dialog are grossly inaccurate. The "session input tokens" counter grows by the full context size per turn rather than just the new content added. This misleads users about their actual token consumption and costs.
- **Suggested fix**: Track only the delta of new content tokens, not the full `prompt_tokens`. The current max+delta pattern correctly deduplicates within a request, but the delta between requests should only count newly-added message content, not the repeated system prompt + history that grows each turn. Alternative: use `prompt_tokens` only from the first Usage event per session, or track cumulative via character estimates of user messages.
- **Verified**: ✓ Agent traced call path — `AgentEvent::Usage` → `process_agent_events()` → `app.accumulate_token_usage()` fires on every SSE usage chunk. With `stream_options.include_usage = true`, each streaming phase emits a Usage event with full `prompt_tokens`.

---

## High Findings

### H-1 Bash tool: stdout/stderr reads may outlive killed child on timeout
- **Location**: `src/tools/bash.rs:107-155` — `execute()` (lines ~107-155)
- **Issue**: When the timeout fires, `child.kill().await` is called, but `stdout_fut` and `stderr_fut` (which hold internal references to `child.stdout` and `child.stderr`) may still be running. The `tokio::join!` in the select branch completes, but if the timeout branch wins, the futures are dropped while potentially holding file descriptors. More critically, the `result` variable is `None` on cancellation, so partial output from killed streams is silently discarded — this is correct for cancellation but the timeout path has a subtle issue: `exit_result` is `Err(_)` (timeout), then `child.kill().await` runs, but the stdout/stderr futures from `tokio::join!` already completed and their data is in `stdout_output`/`stderr_output`. This is actually handled correctly — the combined output IS captured. However, if the child's stdout pipe hasn't been fully read when killed, there's a potential deadlock where the child blocks on a full pipe buffer.
- **Why it matters**: Under heavy output (commands producing >64KB before timeout), the child process can block indefinitely writing to a full pipe buffer while the reader is racing with the kill signal. This manifests as hangs during timeout scenarios.
- **Suggested fix**: Use `tokio::process::Child::kill()` before awaiting stream reads, or use `wait_with_output()` pattern which handles this correctly. The current pattern of reading streams concurrently with the wait is sound for normal exits but has edge cases on timeout.
- **Verified**: ✓ Agent traced call path — `tokio::select!` races `tokio::join!(stdout_fut, stderr_fut, wait_fut)` against `cancel_fut`. On timeout, `wait_fut` returns `Err(Elapsed)`, then `child.kill().await` runs while stdout/stderr futures may still have unread pipe data.

### H-2 Stop hook blocked restart feeds raw error string as user message
- **Location**: `src/agent_events.rs:460-510` — `Done(Ok(()))` handler (lines ~460-510)
- **Issue**: When a Stop hook blocks (exit code 2), the `blocked_error` (stderr from the hook script) is passed directly as the user message to `run_agent_with_snapshot()`. This raw error string becomes the LLM's next instruction. If the hook's stderr contains special characters, shell metacharacters, or is very long, it could confuse the model or produce unexpected behavior.
- **Why it matters**: The LLM receives an unstructured error message as its prompt, which may not lead to productive continuation. A better approach would be to wrap it in a structured format like "The agent was prevented from stopping because: {error}. Please continue working on the current task."
- **Suggested fix**: Wrap `stop_blocked_error` in a structured wrapper before passing to the agent: `format!("Agent was blocked from stopping. Reason: {}\n\nContinue your current work.", stop_blocked_error)`.
- **Verified**: ✓ Agent traced call path — Stop hook → exit code 2 → `blocked_error = result.stderr` → `app.start_streaming(&stop_blocked_error)` → `run_agent_with_snapshot(..., stop_blocked_error, ...)`.

### H-3 `once_cell`, `fuzzy-matcher`, and `syntect` are unused dependencies
- **Location**: `Cargo.toml:23,28,41`
- **Issue**: Three production dependencies are declared but never imported in any source file:
  - `once_cell = "1"` — no `use once_cell` found anywhere (Rust 2024's `std::sync::LazyLock` is used instead)
  - `fuzzy-matcher = "0.3"` — no `fuzzy_matcher::` imports found; fuzzy matching is implemented manually in `edit_diff.rs`
  - `syntect = { version = "5", features = ["default-onig"] }` — no `use syntect` found anywhere; syntax highlighting appears to be handled by ratatui-md
- **Why it matters**: Unused dependencies increase compile time, binary size, and supply chain attack surface. `syntect` with `default-onig` is particularly heavy (pulls in Oniguruma regex bindings).
- **Suggested fix**: Remove all three from `Cargo.toml`. Run `cargo build` to confirm no breakage.

### H-4 `pulldown-cmark` duplicated in root crate (transitive via ratatui-md)
- **Location**: `Cargo.toml:30`
- **Issue**: `pulldown-cmark = "0.12"` is listed as a direct dependency of the root crate, but it's only used by `crates/ratatui-md`. The root crate has no `use pulldown_cmark` imports. This creates a duplicate direct dependency when it's already pulled in transitively.
- **Why it matters**: Duplicate dependencies can cause version conflicts and increase compile time unnecessarily.
- **Suggested fix**: Remove `pulldown-cmark` from the root `Cargo.toml`. It's already a dependency of `ratatui-md` which is a workspace member.

---

## Medium Findings

### M-1 Token tracker: `apply_provider_usage` overwrites `turn_tokens` instead of accumulating
- **Location**: `src/context/token_tracker.rs:130-137` — `apply_provider_usage()` (lines ~130-137)
- **Issue**: In multi-phase turns (LLM → tools → LLM), each phase emits a Usage event. `apply_provider_usage()` correctly accumulates into `provider_completion_tokens += completion_tokens`, but then overwrites `turn_tokens = completion_tokens` (the single-phase value, not the accumulated total). If `has_provider_usage` is false at `finalize_turn()` time (shouldn't happen normally), `turn_tokens` would be stale.
- **Why it matters**: Under normal operation `finalize_turn()` prefers `provider_completion_tokens`, so this is masked. But if provider usage data is lost or the turn ends before Usage arrives, the displayed token count for that turn would be wrong (shows only the last phase's tokens, not the total).
- **Suggested fix**: Change line 136 from `self.turn_tokens = completion_tokens` to `self.turn_tokens = self.provider_completion_tokens` so `turn_tokens` stays in sync with the accumulated value.

### M-2 SSE parser sends `Thinking(true)` on every reasoning chunk, causing potential flicker
- **Location**: `src/provider/sse.rs:157-161` — `process_sse_chunk()` (lines ~157-161)
- **Issue**: When a reasoning chunk arrives, the code sends `ChatChunk::Thinking(true)` unconditionally. This means every reasoning delta triggers a "thinking started" event, even though thinking is already active. The TUI's `ThinkingStart` handler adds a new thinking message if one doesn't exist, so repeated `Thinking(true)` events could cause duplicate thinking messages in edge cases.
- **Why it matters**: If the SSE parser processes chunks rapidly, multiple `ThinkingStart` events could arrive before the TUI processes the first one, potentially creating duplicate thinking message entries in the chat.
- **Suggested fix**: Track whether thinking is already active in `ParseState` and only send `Thinking(true)` on the first reasoning chunk: add `let was_reasoning = state.reasoning_received; state.reasoning_received = true; if !was_reasoning { tx.send(ChatChunk::Thinking(true)).await; }`.

### M-3 `serde_json::to_value(&entry).unwrap_or_default()` silently produces empty JSON on serialization failure
- **Location**: `src/session/manager.rs:443` — `save_token_rate()` (lines ~441-447)
- **Issue**: If `TokenRateEntry` fails to serialize (extremely unlikely but possible if the struct changes), `.unwrap_or_default()` produces an empty `serde_json::Value` (null-like), which gets written to the session file. This corrupts the JSONL with a custom entry that has no usable data.
- **Why it matters**: Silent data corruption in session files. The corrupted entry won't cause crashes (it's filtered by `custom_type == "token-rate"`) but wastes disk space and could confuse diagnostics.
- **Suggested fix**: Use `.unwrap()` (will never fail for a well-defined struct) or propagate the error: `let data = serde_json::to_value(&entry).expect("TokenRateEntry serialization failed");`.

### M-4 Cursor backspace `unwrap()` can panic on edge case with mid-character positions
- **Location**: `src/tui/editor/cursor.rs:178-190` — `cursor_backspace()` (lines ~178-190)
- **Issue**: The function calls `prev_char_offset(input, *pos).unwrap()` when `*pos > 0` and `input` is non-empty. However, `prev_char_offset` returns `None` when `safe_pos == 0`, which can happen if `*pos` falls within a multi-byte character at the very start of the string (e.g., pos=1 for a 2-byte emoji at index 0). The early guards (`*pos == 0` and `input.is_empty()`) don't cover this case.
- **Why it matters**: A panic in the editor during input could crash the TUI mid-session, losing unsaved work.
- **Suggested fix**: Replace `.unwrap()` with a guard: `let Some((offset, width)) = prev_char_offset(input, *pos) else { return; };`.

### M-5 Compaction `find_cut_point` returns `i+1` which can skip valid entries at boundary
- **Location**: `src/session/compaction.rs:379-386` — `find_cut_point()` (lines ~379-386)
- **Issue**: When the accumulated tokens exceed `keep_recent_tokens`, the function returns `Some(std::cmp::min(i + 1, end_index))`. Entry `i` is the one that pushed over the budget, and `i+1` means entry `i` gets compacted. However, if entry `i` is a tool result (which was skipped via `continue` earlier), `i+1` could land on the next assistant message, creating a valid cut. But if entry `i` is an assistant message, it gets compacted even though it contributed to exceeding the budget — this is actually correct behavior (the entry that exceeds should be removed). The real issue: when `i` is the last entry in the range (`end_index - 1`), `i+1 == end_index` means nothing is kept, which violates `MIN_KEEP_MESSAGES`.
- **Why it matters**: In edge cases with very few messages between compaction boundaries, the cut point could eliminate all messages, causing compaction to fail with a confusing error rather than gracefully skipping.
- **Suggested fix**: Add a guard before returning: ensure `i + 1 < end_index` or fall through to `last_valid_cut`.

---

## Low Findings

### L-1 Duplication: auto-scroll guard pattern repeated ~20 times
- **Location**: `src/agent_events.rs` — throughout event handlers
- **Issue**: The pattern `if !app.chat.pinned_scroll { app.chat.auto_scroll = true; }` appears in nearly every event handler (Token, ThinkingStart, ToolCallStart, RetryStart, etc.). This is ~20 repetitions of identical 3-line blocks.
- **Why it matters**: Increases code size and maintenance burden. If the auto-scroll logic changes, every occurrence must be updated.
- **Suggested fix**: Extract to `app.maybe_auto_scroll()` helper method: `fn maybe_auto_scroll(&mut self) { if !self.chat.pinned_scroll { self.chat.auto_scroll = true; } }`.

### L-2 Duplication: compaction restart logic duplicated in agent_events.rs
- **Location**: `src/agent_events.rs:100-165` and `src/agent_events.rs:468-510`
- **Issue**: The code to restart the agent after compaction (compaction success path) and after stop-hook block (stop blocked path) shares ~40 lines of nearly identical logic: building context messages, spawning `run_agent_with_snapshot`, replacing channels. This is copy-paste with minor variable name differences.
- **Why it matters**: Changes to the restart logic (e.g., adding new parameters) must be applied in two places, increasing risk of inconsistency.
- **Suggested fix**: Extract to `fn restart_agent(app: &mut App, agent_handle: &mut ..., cancel_tx: &mut ..., rx: &mut ..., user_message: String) -> ...` helper.

### L-3 `pulldown-cmark` in root `Cargo.toml` is redundant (already covered by H-4)
- **Location**: `Cargo.toml:30`
- **Issue**: See H-4 for details. Listed separately as Low because the impact is minor (no functional bug, just dependency hygiene).
- **Suggested fix**: Remove from root `Cargo.toml`.

### L-4 Status bar auto-scroll guard in `handle_compaction_result` repeated 4 times
- **Location**: `src/event_loop.rs:100-155` — `handle_compaction_result()` (lines ~100-155)
- **Issue**: The pinned scroll guard pattern is duplicated across Success, Failed, and Panicked branches — each has the same 4-line conditional block.
- **Why it matters**: Minor DRY violation within a single function.
- **Suggested fix**: Extract to a local closure or call `app.maybe_auto_scroll()` (see L-1).

---

## Dependency Audit

| Dependency | Status | Notes |
|------------|--------|-------|
| `anyhow` | ✓ Used | Error handling throughout |
| `async-openai` | ✓ Used | Provider layer |
| `async-trait` | ✓ Used | ToolHandler trait |
| `chrono` | ✓ Used | Timestamps, session entries |
| `crossterm` | ✓ Used | Terminal I/O |
| `dashmap` | ✓ Used | File mutation queue |
| `dirs` | ✓ Used | Config directory |
| `fastrand` | ✓ Used | Retry jitter |
| `futures-util` | ✓ Used | EventStream |
| `fuzzy-matcher` | ✗ **Unused** | No imports found; manual fuzzy matching in edit_diff.rs |
| `ignore` | ✓ Used | .gitignore parsing |
| `mimalloc` | ✓ Used | Global allocator |
| `once_cell` | ✗ **Unused** | Replaced by `std::sync::LazyLock` (Rust 2024) |
| `partial-json-fixer` | ✓ Used | Tool call argument repair |
| `pulldown-cmark` | ⚠ **Duplicate** | Direct dep in root + transitive via ratatui-md. Root has no imports. |
| `ratatui` | ✓ Used | TUI framework |
| `regex` | ✓ Used | Various pattern matching |
| `reqwest` | ✓ Used | HTTP client for provider |
| `serde` | ✓ Used | Serialization throughout |
| `serde_json` | ✓ Used | JSON handling |
| `serde_yaml` | ✓ Used | Skill metadata parsing |
| `similar` | ✓ Used | Diff generation |
| `syntect` | ✗ **Unused** | No imports found; syntax highlighting via ratatui-md |
| `tachyonfx` | ✓ Used | Animation effects |
| `thiserror` | ✓ Used | Error types |
| `tokio` | ✓ Used | Async runtime |
| `tracing` | ✓ Used | Logging |
| `tracing-appender` | ✓ Used | Log file rotation |
| `tracing-subscriber` | ✓ Used | Log subscription |
| `unicode-normalization` | ✓ Used | Fuzzy text matching |
| `unicode-width` | ✓ Used | Display width calculations |
| `uuid` | ✓ Used | Session/message IDs |
| `walkdir` | ✓ Used | File discovery |

**Summary**: 3 unused dependencies (`once_cell`, `fuzzy-matcher`, `syntect`), 1 duplicate (`pulldown-cmark` in root). All other dependencies are actively used.
