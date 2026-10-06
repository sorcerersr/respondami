# Fix: Shortcut characters leaking into input area

**Date:** 2026-07-30
**Status:** Approved

## Problem

When pressing a shortcut like `Ctrl+O` (toggle thinking blocks), the base character (`o`) appears in the input area as if it was typed normally. Same issue affects `Ctrl+T`, `Ctrl+/`, and any future `Ctrl+char` shortcuts.

## Root Cause

The key handler chain uses `bool` return types on its early-return gateways:
- `true` = quit
- `false` = everything else (handled OR unhandled)

When `ModalLayer::handle()` consumes a shortcut via `handle_ui_shortcuts()`, it returns `Ok(false)`. Back in `handle_key_event()`, the check `if modal_layer.handle(...) { return Ok(true); }` doesn't trigger, so the event falls through to `IdleHandler` → `InputLayer`, which inserts the character into the input buffer.

The return type has two states but needs three: **Quit**, **Handled**, **Unhandled**.

## Design

### New enum

```rust
pub enum KeyEventResult {
    Quit,
    Handled,
    Unhandled,
}
```

Added to `src/key_handler/mod.rs` alongside the existing `StateHandler` enum.

### Updated callers

In `handle_key_event()`, both gateway calls become `match` arms:

```rust
// 1. Truly global shortcuts
match global::handle_global_shortcuts(app, key, terminal).await? {
    KeyEventResult::Quit => return Ok(true),
    KeyEventResult::Handled => return Ok(false),
    KeyEventResult::Unhandled => {}
}

// 2. Modal-aware global shortcuts
match modal_layer.handle(app, key)? {
    KeyEventResult::Quit => return Ok(true),
    KeyEventResult::Handled => return Ok(false),
    KeyEventResult::Unhandled => {}
}
```

### Updated return types

| Function | Old return | New return |
|---|---|---|
| `global::handle_global_shortcuts()` | `bool` | `KeyEventResult` |
| `ModalLayer::handle()` | `bool` | `KeyEventResult` |

- Ctrl+D → `KeyEventResult::Quit`
- Consumed shortcut (Ctrl+O, Ctrl+T, PgUp/PgDown) → `KeyEventResult::Handled`
- No match → `KeyEventResult::Unhandled`

### Scope

Files changed: 4

| File | Change |
|---|---|
| `src/key_handler/mod.rs` | Add enum, update `handle_key_event()` |
| `src/key_handler/global.rs` | Return `KeyEventResult` |
| `src/key_handler/layers/modal.rs` | Return `KeyEventResult` |
| `src/key_handler/layers/streaming_ui_tests.rs` | Update test assertions |

Not changed: `InputLayer`, `StateTransitionLayer`, `NavigationLayer`, all state handlers, streaming event loop.

### Testing

Existing tests cover shortcut paths. Assertions updated from `true`/`false` to enum variants. No new tests needed — behavior is unchanged except the bug is fixed.
