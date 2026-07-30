# Unblock UI Shortcuts During Streaming — Design

## Problem

While the application is receiving streaming data from the LLM, keyboard shortcuts for purely UI features are blocked. The `process_agent_events()` inner event loop in `src/agent_events.rs` only handles `PgUp`/`PgDown` (scroll) and `Ctrl+C`/`Esc` (cancel). All other shortcuts are dropped via `_ => {}`.

Affected shortcuts:
- `F1` — help dialog
- `Ctrl+O` / `Ctrl+/` — toggle thinking/reasoning display
- `Ctrl+T` — toggle tool output expand/collapse

These are pure UI operations that don't interact with the agent loop and should be available at all times.

## Design

### Architecture

Extract the three UI-toggle shortcuts from `ModalLayer` into a standalone function `handle_ui_shortcuts()` in a new module. Both call sites delegate to it:

- **`ModalLayer`** — replaces inline F1/Ctrl+O/Ctrl+T logic with `handle_ui_shortcuts()`
- **`process_agent_events()`** — calls `handle_ui_shortcuts()` before the catch-all

### Components

| File | Change |
|------|--------|
| `src/key_handler/layers/streaming_ui.rs` | **New** — `handle_ui_shortcuts(app, key) -> Result<bool>` |
| `src/key_handler/layers/mod.rs` | Add `pub mod streaming_ui;` |
| `src/key_handler/layers/modal.rs` | Replace 3 inline blocks with `streaming_ui::handle_ui_shortcuts()` |
| `src/agent_events.rs` | Add `streaming_ui::handle_ui_shortcuts()` call in terminal event drain loop |
| `src/key_handler/layers/streaming_ui_tests.rs` | **New** — unit tests for each shortcut |

### Data Flow

```
Terminal key event during streaming
    → process_agent_events() inner event loop
        → handle_ui_shortcuts(app, key)
            ├─ F1 → app.modal.state = AppState::HelpPopup
            ├─ Ctrl+O/Ctrl+/ → toggle thinking_display
            ├─ Ctrl+T → toggle tool output
            └─ other → fall through to existing handlers (scroll, cancel)
```

### Shortcuts unblocked

| Shortcut | Action | Safe during streaming? |
|----------|--------|----------------------|
| `F1` | Open help popup | Yes — pure UI overlay |
| `Ctrl+O` / `Ctrl+/` | Toggle thinking display | Yes — config flag only |
| `Ctrl+T` | Toggle tool output | Yes — UI state only |

### Shortcuts remaining blocked

| Shortcut | Reason |
|----------|--------|
| `Ctrl+G` | Command palette can trigger agent-dependent actions |
| `Enter`, character input | Editor is inactive during streaming |

### Error Handling

`handle_ui_shortcuts()` returns `anyhow::Result<bool>` for consistency. Config save failures propagate up, same as existing `ModalLayer` behavior.

### Testing

- Unit tests for each shortcut in `streaming_ui_tests.rs`
- Existing `modal_tests.rs` tests continue passing (behavior unchanged)
- Integration test in `agent_events_tests.rs` for streaming + UI shortcut coexistence
