# Token Stats — Non-blocking Dialog Design

**Date:** 2026-07-30

## Problem

`compute_project_stats()` performs synchronous file I/O on the TUI main thread, blocking the UI while scanning session files. For projects with many sessions this could freeze the UI briefly.

## Solution

Spawn the computation on `tokio::task::spawn_blocking`, open the dialog immediately with an animated "Gathering metrics…" loading indicator, then poll for completion in the main event loop (same pattern as compaction).

## State Model

### New field on `App`

```rust
pub token_stats_task: Option<tokio::task::JoinHandle<ProjectTokenStats>>,
```

## Data Flow

### Command executor (`"token_stats"` branch)

1. Spawn `compute_project_stats(cwd)` via `tokio::task::spawn_blocking`
2. Store `JoinHandle` in `app.token_stats_task`
3. Set `app.modal.state = AppState::TokenStatsDialog`
4. Return immediately — main loop redraws with loading state

### Main loop polling (`lib.rs`)

```rust
if let Some(task) = app.token_stats_task.as_mut()
    && task.is_finished()
{
    let handle = app.token_stats_task.take().unwrap();
    let stats = handle.await.unwrap_or_default();
    app.modal.token_stats = Some(stats);
}
```

### Render function

`render_token_stats_dialog` checks `app.modal.token_stats`:

- **`None` (loading):** Panel shows `"Gathering metrics…"` with sweep animation
- **`Some(stats)` (ready):** Existing stats display (sessions, totals, averages, I/O ratio)

### Animation wiring

Extend `tick_activity_indicator()` in `event_loop.rs` to also tick when token stats dialog is in loading state:

```rust
let label = if app.is_working() {
    // existing working labels...
} else if app.modal.state == AppState::TokenStatsDialog
    && app.token_stats_task.is_some()
{
    Some("Gathering metrics…")
} else {
    None
};
```

### Esc dismissal during loading

`TokenStatsHandler` clears both `modal.token_stats` and `app.token_stats_task` so no dangling `JoinHandle` remains.

## Error Handling

- If `JoinHandle` panics or I/O fails → `handle.await.unwrap_or_default()` → dialog shows `"No session data found"` and user dismisses with Esc
- No system message needed — dialog communicates the result

## Files Modified

| File | Change |
|------|--------|
| `src/tui/app.rs` | Add `token_stats_task` field, update `Debug` impl |
| `src/commands.rs` | Spawn blocking task instead of synchronous call |
| `src/lib.rs` | Poll `token_stats_task` in main loop |
| `src/tui/layout.rs` | Render loading state when `token_stats.is_none()` |
| `src/event_loop.rs` | Extend `tick_activity_indicator` for loading case |
| `src/key_handler/token_stats.rs` | Also clear `token_stats_task` on dismiss |
| `src/tui/token_stats_tests.rs` | Add test for loading→ready transition |

## Not Done

- Auto-close timer after "no data found" — YAGNI, user can press Esc
