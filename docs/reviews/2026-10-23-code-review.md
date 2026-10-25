# Code Health Check — 2026-10-23

## Scope

- **Files**: 189 `.rs` files
- **LOC**: 41,315
- **Crates**: 3 (`respondami` root, `ratatui-widgets`, `ratatui-md`)
- **Focus**: Correctness (Pass 1), code quality (Pass 2), dependency audit (Pass 3)
- **Methodology**: Three passes in dependency order. Every file in scope was read in full and evaluated against the pass rules. Pass 1 (correctness) and Pass 2 (quality) were evaluated in a single read per batch to avoid re-review drift; findings are tagged P1/P2 accordingly. Test files received a lighter pass (dead code and duplication only), per the skill's exclusion of test coverage.

## Summary

| Severity | Count |
|----------|-------|
| Critical | 0 |
| High     | 0 |
| Medium   | 7   |
| Low      | 36  |
| **Total**| **43**|

No data-loss, corruption, security, or crash-in-normal-usage defects were found. The seven Medium findings are rare-condition correctness issues and one deprecated dependency. The prior review's (2026-07-30) Critical and High findings have all been verified fixed (see "Prior Findings" below).

---

## Prior Findings (2026-07-30) — Verification

| ID  | Location | Status |
|-----|----------|--------|
| C-1 | `app.rs` `accumulate_token_usage` double-count | **FIXED** (max+delta + `reset_request_usage`) |
| H-1 | `bash.rs` timeout pipe deadlock | **FIXED** (spawned abortable stream readers; process-group kill; handles awaited before timeout branch) |
| H-2 | `agent_events.rs` raw stop-hook error as prompt | **FIXED** (`wrapped_error` framing) |
| M-1 | `tracker.rs` `apply_provider_usage` overwrite | **FIXED** (now accumulates `+=`) |
| M-2 | `sse.rs` `Thinking(true)` every chunk | **FIXED** (gated on `reasoning_received`) |
| M-3 | `serde_json::to_value` `unwrap_or_default` | Re-examined; see L-3 |
| M-4 | `cursor.rs` backspace unwrap | Re-examined; cursor.rs clean this pass |
| M-5 | compaction `find_cut_point` i+1 boundary | **STILL PRESENT** → M-6 below |
| L-1 | auto-scroll guard duplication ~20x | Likely fixed (`maybe_auto_scroll` consolidation) |
| L-2 | compaction restart duplication | Re-examined this pass |
| L-4 | `handle_compaction_result` auto-scroll x4 | Likely fixed (single `pinned_scroll_guard` param) |

Dependency items from the prior review (once_cell, fuzzy-matcher, syntect, root pulldown-cmark) were already removed from `Cargo.toml`.

---

## Findings

### Medium

#### [M-1] `is_animated_gif` false positive rejects valid static GIFs
- **Location**: `src/image.rs` — `is_animated_gif`
- **Issue**: The check counts `0x2C` bytes anywhere in the file. A single-frame GIF whose palette or LZW data happens to contain `0x2C` bytes is misdetected as having a second frame descriptor.
- **Why it matters**: A valid static image is rejected with `Err(Unsupported)`, so the `read` tool refuses to attach it. Triggered by specific byte content, not by extension.
- **Suggested fix**: Walk the GIF block structure properly — iterate image descriptors (`0x2C`), graphic-control extensions (`0x21 0xF9`), and stop at the trailer (`0x3B`), skipping color tables and LZW sub-blocks. Count only genuine image descriptors.

#### [M-2] SSE debug capture lost on mid-turn agent restart
- **Location**: `src/sse_debug.rs` (global `CURRENT_TURN` slot) + `src/agent_events.rs` `process_agent_events`
- **Issue**: On a mid-turn restart (compaction or stop-hook), the code does `sse_guard = new_agent.sse_guard`. The new guard is assigned first, then the old guard drops and sets the global `CURRENT_TURN = None`, wiping the new guard's registration.
- **Why it matters**: After a compaction/stop-hook restart, SSE captures are silently lost for the rest of the turn. Debug-only feature, so no user-facing data loss, but the capture is incomplete.
- **Suggested fix**: Drop the old guard before installing the new one, or make the global slot a `Rc`/generation counter that the new guard overwrites atomically.

#### [M-3] Interleaved thinking phases diverge display from persisted blocks
- **Location**: `src/provider/sse.rs` — `process_sse_chunk`
- **Issue**: `Thinking(true)` is sent only when `!state.reasoning_received` (once per stream). A second thinking phase within one response (interleaved reasoning) emits `Reasoning` deltas without a fresh `ThinkingStart`.
- **Why it matters**: `streaming.rs` drops those deltas from the persisted blocks (the last block is `Text`), while the TUI appends them to the first thinking block. Display and persistence diverge. Rare — only interleaved-thinking models.
- **Suggested fix**: Track thinking-phase transitions (Text→Reasoning) and emit `ThinkingStart`/`ThinkingEnd` on each transition, not just the first.

#### [M-4] Mixed exact+fuzzy edits replace in the wrong region
- **Location**: `src/tools/edit_diff.rs` — `apply_edits`
- **Issue**: When a single edit call mixes an exact-match edit with a fuzzy-normalized edit, the base content becomes the fuzzy-normalized content, but the exact edit's index is computed in original-content space.
- **Why it matters**: The exact edit's offset is applied to the wrong coordinate space, so the replacement lands in the wrong region (silent corruption) or errors. Rare: requires one `oldText` to match exactly and another to need fuzzy normalization in the same call.
- **Suggested fix**: Compute all edit offsets against the same base (either all original or all normalized), or reject mixed exact/fuzzy calls with a clear error.

#### [M-5] `build_context_from_entries` drops all post-cut messages on non-Message cut
- **Location**: `src/session/manager.rs` — `build_context_from_entries`
- **Issue**: `skip_until` is only cleared in the `Message` arm. If `first_kept_entry_id` points at a non-`Message` entry (`ModelChange`/`Compaction`), the skip is never cleared and every message after the cut is silently dropped.
- **Why it matters**: Reachable: `find_cut_point`/`find_cut_point_by_count` (llm.rs) and `find_budget_cut_index` snap (cut.rs) can both return a `ModelChange` index, making `first_kept_entry_id` a `ModelChange` id. The rebuilt context then loses the entire tail.
- **Suggested fix**: Clear `skip_until` on any entry whose id matches `first_kept_entry_id`, not only `Message` entries.

#### [M-6] `find_cut_point` early return can orphan a tool result
- **Location**: `src/session/compaction/llm.rs` — `find_cut_point`
- **Issue**: The early return `i+1` (first kept entry) can land on a `Tool` result. The budget-crossing entry `i` is an assistant tool-call, and `i+1` is its tool result.
- **Why it matters**: The kept prefix starts with an orphaned tool result whose assistant tool-call was compacted away, breaking the `assistant→tool` alternation and risking provider rejection. (Carried over from the 2026-07-30 M-5.)
- **Suggested fix**: After selecting `i+1`, advance the cut past any leading `Tool` entries until the first `Assistant`/`User`/`Message` entry.

#### [M-7] `serde_yaml` 0.9 is deprecated
- **Location**: `Cargo.toml` (root) — `serde_yaml = "0.9"`
- **Issue**: The dependency resolves to `0.9.34+deprecated`. The crate is unmaintained.
- **Why it matters**: Used in `src/skills.rs` for frontmatter parsing. A deprecated, unmaintained parser is a long-term maintenance and security risk.
- **Suggested fix**: Migrate to `serde_yml` (a maintained fork with a compatible API) and update the frontmatter parsing call sites.

### Low

#### [L-1] `token_stats` swallows compute errors
- **Location**: `src/commands.rs` — `token_stats`
- **Issue**: `compute_project_stats(...).unwrap_or_default()` inside `spawn_blocking` swallows errors.
- **Why it matters**: The dialog silently shows empty stats instead of an error.
- **Suggested fix**: Propagate the error and surface a message in the dialog.

#### [L-2] Config: literal `~` never expanded; `context_window` truncation
- **Location**: `src/config.rs`
- **Issue**: `dirs::config_dir().unwrap_or_else(|| PathBuf::from("~/.config"))` uses a literal `~` that is never expanded (only reached when `HOME` is unset). Separately, `context_window` does `v as u32`, silently truncating values > 4G.
- **Why it matters**: The fallback path is wrong on systems without `HOME`; oversized `context_window` values wrap to a small number.
- **Suggested fix**: Use a real home lookup for the fallback; validate/saturate `context_window` instead of a blind cast.

#### [L-3] `message_wire_value` masks serialization drift
- **Location**: `src/history_guard.rs` — `message_wire_value`
- **Issue**: `to_value(msg).unwrap_or(Value::Null)` — a message that fails to serialize digests as `"null"`.
- **Why it matters**: The history guard (log-only) would treat two different unserializable messages as identical, masking real drift. Low impact because the guard never blocks.
- **Suggested fix**: Log a warning and use a digest of the raw bytes on serialization failure.

#### [L-4] `undo_turn_setup` leaves an empty session file on disk
- **Location**: `src/turn.rs` — `undo_turn_setup`
- **Issue**: Rollback replaces the store with `SessionStore::new`, but the auto-created empty `.jsonl` file remains on disk.
- **Why it matters**: Orphaned empty session files accumulate.
- **Suggested fix**: Remove the file on rollback if it was created during the turn.

#### [L-5] Endpoint-check result shown only if it finished before the first frame
- **Location**: `src/lib.rs`
- **Issue**: The endpoint check result is displayed only via a single `try_recv` before the first frame; otherwise it is never shown in the TUI (only a `warn!` log).
- **Why it matters**: A slow endpoint check's result is invisible to the user.
- **Suggested fix**: Keep the pending result and render it on the next frame tick.

#### [L-6] `token_stats` polling block copy-pasted; second is dead code
- **Location**: `src/lib.rs` (~155-169)
- **Issue**: The token-stats polling block appears twice in a row; the second copy is unreachable.
- **Why it matters**: Dead code that misleads readers.
- **Suggested fix**: Delete the second copy.

#### [L-7] `PartialToolCall` defined twice
- **Location**: `src/provider/mod.rs:381` and `src/tui/agent_event.rs:10`
- **Issue**: Two identical `PartialToolCall` definitions with the same fields.
- **Why it matters**: Duplication; they can drift independently.
- **Suggested fix**: Define it once and re-export.

#### [L-8] Empty root `AGENTS.md` shadows `.respondami/AGENTS.md`
- **Location**: `src/agents_md.rs`
- **Issue**: An empty root `AGENTS.md` returns `Ok(None)` without falling through to `.respondami/AGENTS.md`.
- **Why it matters**: A zero-byte root file silently disables the project's real AGENTS.md.
- **Suggested fix**: Treat an empty root file as absent and check the hidden location.

#### [L-9] `logging.rs` const typo + stale rotation doc
- **Location**: `src/logging.rs`
- **Issue**: Const is named `RESPORDAMI_DIR` (transposed). The doc comment describes size-based rotation (1MB, 5 files) but the code does daily rotation (7 files).
- **Why it matters**: Misleading name and documentation.
- **Suggested fix**: Rename the const; update the doc to match daily rotation.

#### [L-10] `process_agent_events` always returns `false`; dead `if quit` branch
- **Location**: `src/agent_events.rs` — `process_agent_events`
- **Issue**: The function always returns `false`, so the `if quit` branch in `turn.rs` `launch_agent_turn` is dead code with a misleading doc.
- **Why it matters**: Dead code and a contract that is never honored.
- **Suggested fix**: Either implement quit propagation or remove the return value and the dead branch.

#### [L-11] `toggle_all_tool_output` repeats near-identical arms
- **Location**: `src/tui/app.rs` — `toggle_all_tool_output`
- **Issue**: Five near-identical match arms (Bash/Read/Write/Edit/Unknown) each repeat the `content_lines` check and expanded-set logic.
- **Why it matters**: DRY violation; a change must be made in five places.
- **Suggested fix**: Factor the common logic into a helper and call it per variant.

#### [L-12] `handle_retry` shift can overflow for `max_retries` ≥ 64
- **Location**: `src/agent/mod.rs` — `handle_retry`
- **Issue**: `1u64 << (*retry_count - 1)` panics in debug (wraps in release) if the user configures `max_retries` ≥ 64.
- **Why it matters**: A misconfigured value crashes the agent loop in debug builds.
- **Suggested fix**: Clamp the shift amount or use `checked_shl`.

#### [L-13] `run_agent_with_snapshot` has a wrong copy-pasted clippy reason
- **Location**: `src/agent/mod.rs` — `run_agent_with_snapshot`
- **Issue**: `#[expect(clippy::too_many_arguments, reason = "render signature unified across all message types")]` — the reason string is copy-pasted from an unrelated function.
- **Why it matters**: Misleading suppression rationale.
- **Suggested fix**: Correct the reason string.

#### [L-14] `activate_skill` with empty name persists `"unknown"`
- **Location**: `src/agent/mod.rs` — `execute_tool_call`
- **Issue**: `activate_skill` with an empty name sends `SkillActivation { skill_name: "unknown" }`, which the TUI inserts into `active_skills` and persists to the session header.
- **Why it matters**: A bogus `"unknown"` skill pollutes the active set and the saved session.
- **Suggested fix**: Reject/ignore empty skill names before emitting the event.

#### [L-15] Provider task panic leaves the oneshot unsent
- **Location**: `src/agent/streaming.rs`
- **Issue**: If the provider task panics, the oneshot is never sent, so `if let Ok(Err(e))` is skipped and the partial blocks are treated as a normal completion.
- **Why it matters**: A panic is silently converted into a truncated-but-"successful" response.
- **Suggested fix**: Use a `Drop` guard or `catch_unwind` to send an error on panic.

#### [L-16] `streaming.rs` uses a deep relative path
- **Location**: `src/agent/streaming.rs`
- **Issue**: `super::super::provider::Message::content` instead of `crate::provider::Message::content`.
- **Why it matters**: Fragile to module moves; inconsistent with the rest of the crate.
- **Suggested fix**: Use the `crate::` absolute path.

#### [L-17] `from_utf8_lossy` per network chunk can corrupt split UTF-8
- **Location**: `src/provider/sse.rs`
- **Issue**: `String::from_utf8_lossy(&chunk)` per network chunk: a multi-byte UTF-8 sequence split across chunk boundaries becomes U+FFFD, corrupting the JSON line.
- **Why it matters**: Rare data loss on a parse error when a multibyte character straddles a chunk boundary.
- **Suggested fix**: Buffer partial bytes across chunks (e.g., `StringDecoder`) before decoding.

#### [L-18] `complete_inner` swallows empty `choices`
- **Location**: `src/provider/llamacpp.rs` — `complete_inner`
- **Issue**: `response.choices.first()...unwrap_or_default()` — an empty `choices` array yields a silent empty compaction summary.
- **Why it matters**: A provider returning no choices produces an empty summary with no error.
- **Suggested fix**: Return an error when `choices` is empty.

#### [L-19] `complete_inner` pointless serde round-trip
- **Location**: `src/provider/llamacpp.rs` — `complete_inner`
- **Issue**: `serde_json::from_value(to_value(req)?)` round-trips the request to clone it (comment says "strip stream flag", but `stream` is already `false`).
- **Why it matters**: Wasteful serialize/deserialize with no effect.
- **Suggested fix**: Clone the struct directly or build the request without the round-trip.

#### [L-20] `ping_inner` health check uses model `"dummy"`
- **Location**: `src/provider/llamacpp.rs` — `ping_inner`
- **Issue**: The health check requests model `"dummy"`; an endpoint that requires the real model name reports "unreachable".
- **Why it matters**: Misleading health status for single-model endpoints.
- **Suggested fix**: Use the configured model name for the ping.

#### [L-21] `url`/`api_key` fields duplicate data in `client`
- **Location**: `src/provider/llamacpp.rs`
- **Issue**: `url` and `api_key` fields duplicate data already stored in the `client` (`OpenAIConfig`).
- **Why it matters**: Two sources of truth for the same values.
- **Suggested fix**: Derive from `client` or drop the redundant fields.

#### [L-22] `read` with `offset=0` on an empty file underflows
- **Location**: `src/tools/read.rs`
- **Issue**: `offset=0` on a 0-line file: `o > total_lines` is false (`0 > 0`), then `o - 1` underflows (panic in debug, wrap in release).
- **Why it matters**: A model passing `offset=0` to an empty file panics in debug builds.
- **Suggested fix**: Guard `o == 0` before the decrement, or use `saturating_sub`.

#### [L-23] `ToolResult` struct is dead code
- **Location**: `src/tools/mod.rs`
- **Issue**: The `ToolResult` struct (content + error) has no production or test usage; `ToolOutput` replaced it.
- **Why it matters**: Dead type.
- **Suggested fix**: Remove it.

#### [L-24] `rewrite_command` blocks the async runtime
- **Location**: `src/tools/rtk.rs` — `rewrite_command`
- **Issue**: Blocking `std::fs::create_dir_all` inside an async fn.
- **Why it matters**: Briefly blocks a worker thread (3s-bounded, minor).
- **Suggested fix**: Wrap in `spawn_blocking` or accept the bounded cost with a comment.

#### [L-25] `file_queue` canonicalize fallback keys new files by raw path
- **Location**: `src/tools/file_queue.rs`
- **Issue**: The canonicalize fallback keys new (nonexistent) files by raw path, so two different raw paths to the same new file don't serialize.
- **Why it matters**: Rare race on the same new file reached via different paths.
- **Suggested fix**: Normalize the path (symlinks of the parent) before keying.

#### [L-26] `file_queue` cleanup can free a held permit early
- **Location**: `src/tools/file_queue.rs`
- **Issue**: An entry removed while a permit is held (idle > 5min) lets a new op on the same file run in parallel with the old holder.
- **Why it matters**: Unlikely window where two ops on one file overlap.
- **Suggested fix**: Don't evict an entry that still has a live permit.

#### [L-27] `apply_compaction` rewrites with truncate-then-write
- **Location**: `src/session/manager.rs` — `apply_compaction`
- **Issue**: `rewrite_session_file` uses `File::create` (truncate-then-write); a crash mid-rewrite corrupts the session file.
- **Why it matters**: Crash in the small write window loses the session (in-memory state is fine; `load_session` skips bad lines).
- **Suggested fix**: Write to a temp file and atomically rename.

#### [L-28] `rewrite_session_file` silently skips unserializable entries
- **Location**: `src/session/manager.rs` — `rewrite_session_file`
- **Issue**: `if let Ok(json) = serde_json::to_string(entry)` skips entries that fail to serialize.
- **Why it matters**: Inconsistent with the append path (which propagates); a bad entry is silently dropped from the rewrite.
- **Suggested fix**: Propagate the error or log it.

#### [L-29] Resumed tool results always render `has_error: false`
- **Location**: `src/session/display_adapter.rs` — `build_messages`
- **Issue**: Session tool results always render with `has_error: false`; an errored tool result from a resumed session loses its error styling.
- **Why it matters**: Known limitation (error state not persisted), but the display could infer it from the result text.
- **Suggested fix**: Persist the error flag, or infer it from the result content.

#### [L-30] `append_entry` lacks the cache debug-assert
- **Location**: `src/session/manager.rs` — `append_entry`
- **Issue**: No `debug_assert` cache check, unlike `append_message`.
- **Why it matters**: Inconsistent guard; a cache invariant violation in `append_entry` goes uncaught in debug.
- **Suggested fix**: Add the same `debug_assert`.

#### [L-31] VCC `clip` panics on a mid-multibyte boundary
- **Location**: `src/session/compaction/vcc/normalize.rs` — `clip`
- **Issue**: `text[..max]` panics when `max` falls mid-multibyte character (no `floor_char_boundary`, unlike the rest of the codebase).
- **Why it matters**: Compaction crashes on specific non-ASCII lengths at the 60/150/200-char clip boundaries (brief.rs, sections.rs).
- **Suggested fix**: Use `floor_char_boundary` before slicing.

#### [L-32] VCC `rank` treats every path as both modified and read
- **Location**: `src/session/compaction/vcc/rank.rs` — `select_ranked_brief_blocks`
- **Issue**: `read_files = modified_files.clone()` — every path is scored as both modified (+18) and read (+6), so the distinction is dead.
- **Why it matters**: The modified-vs-read scoring has no effect; ranking is less precise than intended.
- **Suggested fix**: Derive `read_files` from actual read tool calls, not from `modified_files`.

#### [L-33] `load_skills_from_dir` doc/code mismatch on flat SKILL.md
- **Location**: `src/skills.rs` — `load_skills_from_dir`
- **Issue**: The doc comment says flat `SKILL.md` files directly in `dir` are checked, but the code only checks subdirectories (`path.is_dir()`).
- **Why it matters**: Flat SKILL.md files are silently ignored; the doc is wrong.
- **Suggested fix**: Either support flat files or correct the doc.

#### [L-34] `render_status_bar` spawns `git` every frame
- **Location**: `src/tui/status_bar.rs` — `render_status_bar`
- **Issue**: `get_git_branch()` spawns `git rev-parse` on every frame render, with no caching.
- **Why it matters**: A process spawn per frame (~20-50ms) is wasteful. Not a correctness bug.
- **Suggested fix**: Cache the branch and refresh on a timer or on file change.

#### [L-35] `terminal_height` is a dead field
- **Location**: `src/tui/layout.rs` — `render`
- **Issue**: `app.ui.terminal_width` is set from `area.width`, but `app.ui.terminal_height` is never updated (stays at default 24) and never read after init.
- **Why it matters**: Dead field that misleads readers.
- **Suggested fix**: Set it from `area.height` or remove it.

#### [L-36] Test helpers `make_app`/`make_key` duplicated across 5 files
- **Location**: `src/turn_tests.rs`, `src/tui/autocomplete_tests.rs`, `src/key_handler/layers/{navigation,streaming_ui,transitions,modal}_tests.rs`
- **Issue**: `make_app()` (identical `App::new(Config::default(), PathBuf::from("."))`) is duplicated in 5 files; `make_key(code, modifiers)` (identical `KeyEvent` builder) in 5 files.
- **Why it matters**: Test-only duplication; no runtime impact.
- **Suggested fix**: Extract a shared test-util module.

---

## Dependency Audit

| Dependency | Status | Notes |
|------------|--------|-------|
| All 30 root deps | Used | Imported in production code; no unused deps |
| All 3 widget/md deps | Used | `ratatui`, `pulldown-cmark`, `unicode-width` |
| Dev-deps (`serial_test`, `tempfile`) | OK | Only in `*_tests.rs`; no production leakage |
| `serde_yaml` 0.9 | **Deprecated** | Resolves to `0.9.34+deprecated`; see M-7 |
| `chrono` `serde` feature | Unused feature | Only `Utc::now().to_rfc3339()` + `Local::now().format()`; no serde use of chrono types |
| Repository URL | Inconsistent | Root `Cargo.toml` = `github.com/respondami/respondami`; both crate `Cargo.toml`s = `github.com/sorcerersr/respondami` |
| Duplicate functionality | None | Single JSON crate, single async runtime |
| Conflicting version reqs | None | No workspace conflicts |
| Typos / duplicate entries | None | Clean `Cargo.toml` |

---

## Triage Status (2026-10-24)

All 36 "fix now" items are implemented across 7 commits on `next` (clippy clean, 1239 tests green). The 7 deferred items are tracked in `todo.md` (L-32 was already there). See `docs/plans/2026-10-23-code-review-triage-design.md` for the commit map.

| ID  | Status  | Commit  | Note |
|-----|---------|---------|------|
| M-1 | Fixed   | f424ba5 | GIF structural block walk |
| M-2 | Fixed   | d72e9b1 | match-on-drop `TurnCaptureGuard` |
| M-3 | Fixed   | b26c5d2 | `in_thinking` phase transitions |
| M-4 | Fixed   | 8cbe87d | re-resolve exact edits against final base |
| M-5 | Fixed   | 49b9a83 | `skip_until` cleared on any matching entry |
| M-6 | Fixed   | 49b9a83 | advance cut past leading Tool entries |
| M-7 | Fixed   | a142995 | `serde_yaml` → `noyalib` (not `serde_yml`, which is a deprecated shim) |
| L-1 | Fixed   | 18bb768 | `compute_project_stats` → `Result`, surfaced in dialog |
| L-2 | Fixed   | 18bb768 | real home fallback + saturate `context_window` |
| L-3 | Fixed   | 18bb768 | log + raw-byte digest on serialization failure |
| L-4 | Fixed   | 18bb768 | remove empty session file on rollback |
| L-5 | Fixed   | 18bb768 | poll endpoint check `try_recv` each loop iteration |
| L-6 | Fixed   | 18bb768 | delete dead token-stats polling block |
| L-7 | Fixed   | 18bb768 | single `PartialToolCall` definition |
| L-8 | Fixed   | 18bb768 | empty root AGENTS.md falls through |
| L-9 | Fixed   | 18bb768 | rename `RESPORDAMI_DIR` + fix rotation doc |
| L-10 | Fixed  | 18bb768 | remove return value + dead `if quit` branch |
| L-11 | Deferred | —       | DRY only, no bug |
| L-12 | Fixed  | d72e9b1 | clamp retry shift |
| L-13 | Fixed  | d72e9b1 | correct clippy reason string |
| L-14 | Fixed  | d72e9b1 | reject empty skill name |
| L-15 | Fixed  | d72e9b1 | `OneshotGuard` sends `Err` on drop |
| L-16 | Fixed  | d72e9b1 | `crate::` path in streaming.rs |
| L-17 | Fixed  | b26c5d2 | `pending: Vec<u8>` UTF-8 boundary buffering |
| L-18 | Fixed  | b26c5d2 | empty `choices` → error |
| L-19 | Fixed  | b26c5d2 | remove serde round-trip |
| L-20 | Fixed  | b26c5d2 | ping uses configured model |
| L-21 | Fixed  | b26c5d2 | fields kept (async-openai 0.42.2 has no config getters); constraint documented |
| L-22 | Fixed  | 8cbe87d | guard `o == 0` in read |
| L-23 | Fixed  | 8cbe87d | remove dead `ToolResult` |
| L-24 | Fixed  | 8cbe87d | comment bounded blocking cost in rtk |
| L-25 | Deferred | —       | rare race |
| L-26 | Deferred | —       | unlikely window |
| L-27 | Fixed  | 49b9a83 | temp file + fsync + atomic rename |
| L-28 | Fixed  | 49b9a83 | propagate unserializable-entry errors |
| L-29 | Deferred | —       | needs JSONL schema change |
| L-30 | Fixed  | 49b9a83 | `append_entry` cache debug-assert |
| L-31 | Fixed  | 18bb768 | `floor_char_boundary` in VCC `clip` |
| L-32 | Deferred | —       | already in `todo.md` (VCC read tracking) |
| L-33 | Fixed  | a142995 | correct `load_skills_from_dir` doc |
| L-34 | Deferred | —       | perf not correctness; cache invalidation is its own design |
| L-35 | Fixed  | 18bb768 | `terminal_height` from `area.height` |
| L-36 | Deferred | —       | test-only duplication |
| dep-1 | Fixed | a142995 | drop unused `chrono/serde` feature |
| dep-2 | Fixed | a142995 | align root repository URL to `sorcerersr/respondami` |
