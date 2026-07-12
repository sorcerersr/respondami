# Respondami — Agent Memory

Critical decisions and known pitfalls. Read before modifying agent/streaming/TUI code.

## Architecture

### Overview

Rust TUI chat app for AI coding agents. Workspace with 3 crates: main app + 2 widget libraries.

- **138 `.rs` files**, ~25K lines total (17K production, 8K tests)
- **3 crates**: `respondami` (main), `ratatui-widgets` (reusable widgets), `ratatui-md` (markdown rendering)
- **Key deps**: ratatui 0.30 (TUI), crossterm 0.29 (terminal), tokio 1 (async), tachyonfx (animations), mimalloc (allocator)

### Core Patterns

- **Channel-based agent**: `src/lib.rs` spawns `run_agent_with_snapshot()` via `tokio::spawn`, communicates via `mpsc::channel::<AgentEvent>(256)`
- **Top-down scroll model**: `scroll_offset = 0` means viewport at top. `scroll_to_bottom()` sets offset to `max_offset`. See `src/tui/chat_state.rs`.
- **Pinned scroll**: During streaming, user scroll-up sets `pinned_scroll = true`, blocking auto-scroll. Resumes when user scrolls within 3 rows of bottom.
- **Providers**: `src/provider/mod.rs` defines `ChatChunk` enum; implementations in `provider/llamacpp.rs`, SSE parsing in `provider/sse.rs`
- **Tools**: Registry pattern in `src/tools/mod.rs` with `ToolHandler` trait. Tools: bash, read, write, edit, rtk, activate_skill
- **Session**: JSONL persistence in `src/session/manager.rs` with fsync. Compaction via LLM summarization.
- **Hooks**: Shell scripts at lifecycle points (`src/hooks/`). Discovered from `~/.config/respondami/hooks/` and `.respondami/hooks/`. Execute synchronously in agent loop — can inject context (exit 0), block actions (exit 2), or log errors (other codes).
- **State modules**: UI state split into focused modules under `src/tui/` — `chat_state.rs` (messages + scroll), `editor_state.rs` (input buffer, history navigation), `agent_state.rs` (streaming + tool calls), `session_state.rs` (persistence), `modal_state.rs` (popups), `config_state.rs` (thinking/hook display), `ui_state.rs` (animation/effects).
- **Context**: `src/context/token_tracker.rs` — TokenRateTracker for turn lifecycle (start/pause/finalize, char counting, provider correction).

### Data Flow

```
User Input → Key Handler → App State → Agent Loop (tokio::spawn)
                                      ↓
                              Provider (SSE stream)
                                      ↓
                              AgentEvent channel (256 cap)
                                      ↓
                              process_agent_events() → TUI render
                                      ↓
                              Tool execution → back to Agent Loop
```

### Key Handler Chain

Each app state composes layers: `InputLayer` → `NavigationLayer` → `StateTransitionLayer` → `ModalLayer`. Top-level `handle_key_event()` dispatches via `StateHandler` enum (not trait objects).

## Development

### Build, Test, Run

```bash
cargo build                                    # debug build
cargo run                                      # run the app
cargo build --release                          # optimized build (LTO, strip)
cargo test --workspace                         # all tests (918 total)
cargo clippy --all-targets --all-features      # must be clean (0 warnings)
```

### Code Style

- **Edition 2024**, strict clippy pedantic + cargo + complexity + correctness + perf + style + suspicious
- **No inline `#[cfg(test)]` modules** — all tests in separate `*_tests.rs` sibling files
- **Private items for tests**: mark `pub(crate)`, never use `#[cfg(test)]` re-exports
- **Module declaration**: `#[cfg(test)] mod xxx_tests;` in immediate parent `mod.rs` (or `src/lib.rs` for top-level)
- **Clippy must always be clean** — 0 errors and 0 warnings. Enforced by pre-commit hook.

### Pre-commit Checks

Install once per clone:

```bash
./scripts/install-hooks.sh
```

Before committing, run both:

```bash
cargo clippy --all-targets --all-features   # must be clean
cargo test --workspace                       # all tests pass
```

### Test Count

`cargo test --workspace` should report **913 tests** (697 root + 41 ratatui-widgets + 175 ratatui-md). If the count drops, a test file was likely removed or renamed.

## Known Pitfalls

### Scroll Model

- `scroll_offset = 0` = viewport at TOP (not bottom). Inverted from many TUI frameworks.
- `scroll_to_bottom()` sets offset to `get_max_offset()`, not 0.
- Pinned scroll (`pinned_scroll = true`) blocks auto-scroll. Cleared when user scrolls within 3 rows of bottom.

### Agent Event Draining

- `Done` events may arrive while `Token`/`Reasoning`/`Usage` events are still buffered in the 256-cap channel.
- Always call `app.drain_pending_events(&mut rx)` before finalizing a turn to prevent silent data loss.

### Tool Call Alternation in JSONL

- Multi-tool-call responses must save `SaveAssistantMessage` + `SaveToolResult` per tool call to maintain `assistant→tool→assistant→tool` alternation pattern.
- First tool call gets full content blocks (text, thinking, tool call). Subsequent ones get just the tool call block.
- Without per-call saves, JSONL has consecutive `tool→tool` messages, corrupting context rebuild.

### Compaction Pattern

- Uses spawn-and-poll: `tokio::spawn` the compaction task, poll `is_finished()` in event loop.
- Keeps UI responsive during compaction (draws, animates, processes terminal events).
- Auto-compaction restarts the agent with a retry message after success.

### Token Usage Accumulation

- Uses max+delta pattern: takes max of each field within a request (deduplicates SSE usage events), adds only delta between requests.
- `estimate_context_tokens()` prefers actual LLM `prompt_tokens` from last Assistant message, falls back to char estimates + system overhead.

### Provider Abstraction

- Adding a new provider: implement `ProviderTrait`, update `Provider::from_settings()`.
- `ProviderError::is_retryable()` classifies errors — context overflow → compaction, billing → fail, network/429/5xx → retry with exponential backoff + jitter.

### Animation Timing

- Effects use `tachyonfx` with time-based duration (ms since last render), not frame count.
- Render timing tracked in `app.ui.last_render_time`. Effects don't run if `elapsed_ms == 0`.

### Session Persistence

- JSONL format with fsync on every append. Session header on line 1, messages appended.
- `apply_compaction()` rewrites the entire file, clears usage data from kept Assistant messages to prevent stale `prompt_tokens` from triggering double compaction.

## File Map

### Core

| File                     | Purpose                                                                |
| ------------------------ | ---------------------------------------------------------------------- |
| `src/main.rs`            | Entry point, mimalloc global allocator                                 |
| `src/lib.rs`             | Main event loop, terminal setup, animation ticks, compaction polling   |
| `src/agent/mod.rs`       | Agent loop, tool orchestration, retry logic, cooperative cancellation  |
| `src/agent/streaming.rs` | SSE streaming from provider, `AgentResponse` builder                   |
| `src/agent_events.rs`    | Agent event processing, bridges async agent loop with TUI              |
| `src/event_loop.rs`      | Shared helpers: draw frame, animation tick, compaction result handling |
| `src/config.rs`          | Config loading from `~/.config/respondami/config.yaml`                 |
| `src/commands.rs`        | Command palette commands and descriptions                              |

### Provider Layer

| File                       | Purpose                                                         |
| -------------------------- | --------------------------------------------------------------- |
| `src/provider/mod.rs`      | ProviderTrait, ChatChunk enum, ProviderError, tool call parsing |
| `src/provider/llamacpp.rs` | OpenAI-compatible provider (llama.cpp, Ollama, etc.)            |
| `src/provider/sse.rs`      | SSE stream parsing, cooperative cancellation                    |

### Tools

| File                          | Purpose                                         |
| ----------------------------- | ----------------------------------------------- |
| `src/tools/mod.rs`            | ToolRegistry, CancelGuard, ToolHandler trait    |
| `src/tools/bash.rs`           | Bash tool with streaming output and timeout     |
| `src/tools/read.rs`           | Read tool with offset/limit support             |
| `src/tools/write.rs`          | Write tool, creates parent dirs automatically   |
| `src/tools/edit.rs`           | Edit tool with diff-based replacement           |
| `src/tools/edit_diff.rs`      | Unified diff generation for edit tool           |
| `src/tools/activate_skill.rs` | activate_skill tool (intercepted in agent loop) |
| `src/tools/rtk.rs`            | RTK rewrite integration for bash commands       |
| `src/tools/file_queue.rs`     | File queue for batch operations                 |

### Session

| File                             | Purpose                                                           |
| -------------------------------- | ----------------------------------------------------------------- |
| `src/session/mod.rs`             | Module declarations and re-exports                                |
| `src/session/manager.rs`         | SessionStore, JSONL I/O, context building, compaction application |
| `src/session/entry.rs`           | SessionEntry types (Session, Message, Compaction, Custom)         |
| `src/session/compaction.rs`      | CompactionEngine, LLM-based summarization                         |
| `src/session/display_adapter.rs` | Adapts session data for display                                   |

### TUI

| File                            | Purpose                                                                 |
| ------------------------------- | ----------------------------------------------------------------------- |
| `src/tui/mod.rs`                | TUI module declarations and re-exports                                  |
| `src/tui/app.rs`                | App struct (thin facade over sub-structs), message helpers, token usage |
| `src/tui/layout.rs`             | Main layout renderer, popup overlays, animation effects                 |
| `src/tui/chat_state.rs`         | Messages, scrolling, viewport dimensions                                |
| `src/tui/editor_state.rs`       | Input buffer, cursor, autocomplete, history navigation                  |
| `src/tui/agent_state.rs`        | Streaming content, pending tool calls, token tracking                   |
| `src/tui/modal_state.rs`        | App states, popups, command palette                                     |
| `src/tui/config_state.rs`       | Config, model, skills, project context                                  |
| `src/tui/ui_state.rs`           | Animations, terminal dimensions, status bar                             |
| `src/tui/agent_event.rs`        | AgentEvent enum (Token, Reasoning, Usage, ToolCall, etc.)               |
| `src/tui/mode.rs`               | AppState and AutocompleteMode enums                                     |
| `src/tui/status_bar.rs`         | Status bar rendering with activity indicator                            |
| `src/tui/theme.rs`              | Theme definitions (gh_dark default)                                     |
| `src/tui/autocomplete.rs`       | File and skill autocomplete handling                                    |
| `src/tui/activity_indicator.rs` | Animated working indicator                                              |
| `src/tui/thinking_display.rs`   | Thinking/reasoning block display                                        |
| `src/tui/hook_display.rs`       | Hook message display modes                                              |
| `src/tui/tracker.rs`            | Token rate tracker display                                              |

### TUI — Messages

| File                                           | Purpose                               |
| ---------------------------------------------- | ------------------------------------- |
| `src/tui/messages/mod.rs`                      | ChatMessage enum, ChatRenderer        |
| `src/tui/messages/assistant_message.rs`        | Assistant message rendering           |
| `src/tui/messages/user_message.rs`             | User message rendering                |
| `src/tui/messages/system_message.rs`           | System message rendering              |
| `src/tui/messages/thinking_message.rs`         | Thinking/reasoning block rendering    |
| `src/tui/messages/tool_call/mod.rs`            | ToolCallMessage, ToolCallVariant enum |
| `src/tui/messages/tool_call/bash.rs`           | Bash tool call rendering              |
| `src/tui/messages/tool_call/read.rs`           | Read tool call rendering              |
| `src/tui/messages/tool_call/write.rs`          | Write tool call rendering             |
| `src/tui/messages/tool_call/edit.rs`           | Edit tool call rendering              |
| `src/tui/messages/tool_call/unknown.rs`        | Unknown tool call rendering           |
| `src/tui/messages/compaction_message.rs`       | Compaction notification rendering     |
| `src/tui/messages/hook_message.rs`             | Hook execution message rendering      |
| `src/tui/messages/skill_activation_message.rs` | Skill activation message rendering    |
| `src/tui/messages/welcome_screen.rs`           | Welcome screen with keybindings       |

### TUI — Editor

| File                          | Purpose                            |
| ----------------------------- | ---------------------------------- |
| `src/tui/editor/mod.rs`       | Editor module declarations         |
| `src/tui/editor/renderer.rs`  | Prompt input rendering             |
| `src/tui/editor/cursor.rs`    | Cursor management and movement     |
| `src/tui/editor/commands.rs`  | Editor commands (cut, paste, etc.) |
| `src/tui/editor/wrap.rs`      | Text wrapping for input area       |
| `src/tui/editor/discovery.rs` | File discovery for @-autocomplete  |

### Key Handling

| File                                    | Purpose                                |
| --------------------------------------- | -------------------------------------- |
| `src/key_handler/mod.rs`                | Key handler chain, StateHandler enum   |
| `src/key_handler/global.rs`             | Global shortcuts (Ctrl+D)              |
| `src/key_handler/idle.rs`               | Idle state handler (composes layers)   |
| `src/key_handler/init_popup.rs`         | Init popup handler                     |
| `src/key_handler/session_select.rs`     | Session select handler                 |
| `src/key_handler/command_palette.rs`    | Command palette handler                |
| `src/key_handler/help_popup.rs`         | Help popup handler                     |
| `src/key_handler/layers/mod.rs`         | Layer declarations                     |
| `src/key_handler/layers/input.rs`       | Character insertion, cursor movement   |
| `src/key_handler/layers/navigation.rs`  | Up/Down/j/k/Tab navigation             |
| `src/key_handler/layers/transitions.rs` | State transitions (Enter, Esc, Ctrl+C) |
| `src/key_handler/layers/modal.rs`       | Modal-aware global shortcuts           |

### Supporting

| File                           | Purpose                                         |
| ------------------------------ | ----------------------------------------------- |
| `src/agents_md.rs`             | AGENTS.md loading from project root             |
| `src/skills.rs`                | Skill discovery and loading (global + project)  |
| `src/turn.rs`                  | Turn tracking and management                    |
| `src/hooks/mod.rs`             | Hook module declarations                        |
| `src/hooks/loader.rs`          | Hook discovery from config and project dirs     |
| `src/hooks/executor.rs`        | Hook execution with context                     |
| `src/context/mod.rs`           | Context module declarations                     |
| `src/context/token_tracker.rs` | TokenRateTracker with EMA + provider correction |
| `src/logging.rs`               | File-based logging setup                        |
| `src/mouse.rs`                 | Mouse scroll enable/disable, scroll line count  |
| `src/sse_debug.rs`             | SSE debugging utilities                         |

### Widget Crates

| File                                                    | Purpose                                |
| ------------------------------------------------------- | -------------------------------------- |
| `crates/ratatui-widgets/src/lib.rs`                     | Widget crate declarations              |
| `crates/ratatui-widgets/src/filled_header_bar.rs`       | Header bar widget                      |
| `crates/ratatui-widgets/src/panel_overlay.rs`           | Centered panel overlay widget          |
| `crates/ratatui-widgets/src/autocomplete_popup.rs`      | Autocomplete dropdown widget           |
| `crates/ratatui-widgets/src/command_palette_overlay.rs` | Command palette widget                 |
| `crates/ratatui-widgets/src/prompt_editor_overlay.rs`   | Prompt editor widget                   |
| `crates/ratatui-md/src/lib.rs`                          | Markdown rendering entry point         |
| `crates/ratatui-md/src/parsing.rs`                      | Markdown parsing with pulldown-cmark   |
| `crates/ratatui-md/src/height.rs`                       | Height calculation for markdown blocks |

## Test Files

### Root Crate Tests (702 tests)

| Test File                                     | Coverage                                                |
| --------------------------------------------- | ------------------------------------------------------- |
| `src/agent/mod_tests.rs`                      | Agent loop, system prompt building                      |
| `src/agent/token_estimation_tests.rs`         | Token estimation from messages                          |
| `src/agent_events_tests.rs`                   | Agent event processing edge cases                       |
| `src/agents_md_tests.rs`                      | AGENTS.md loading and parsing                           |
| `src/commands_tests.rs`                       | Command palette commands                                |
| `src/config_tests.rs`                         | Config loading and validation                           |
| `src/context/token_tracker_tests.rs`          | Token rate tracking, EMA, provider correction           |
| `src/event_loop_tests.rs`                     | Draw frame, compaction result handling                  |
| `src/hooks/executor_tests.rs`                 | Hook execution, exit codes, context                     |
| `src/hooks/loader_tests.rs`                   | Hook discovery from directories                         |
| `src/key_handler/layers/input_tests.rs`       | Input layer key handling                                |
| `src/key_handler/layers/modal_tests.rs`       | Modal layer shortcuts                                   |
| `src/key_handler/layers/navigation_tests.rs`  | Navigation layer (j/k/Up/Down)                          |
| `src/key_handler/layers/transitions_tests.rs` | State transitions                                       |
| `src/logging_tests.rs`                        | Logging initialization                                  |
| `src/provider/llamacpp_tests.rs`              | LlamaCpp provider, request building                     |
| `src/provider/mod_tests.rs`                   | Provider trait, error classification, tool call parsing |
| `src/provider/sse_tests.rs`                   | SSE parsing, cancellation                               |
| `src/session/compaction_tests.rs`             | Compaction engine, summarization                        |
| `src/session/entry_tests.rs`                  | Session entry serialization                             |
| `src/session/manager_tests.rs`                | Session CRUD, context building, compaction application  |
| `src/skills_tests.rs`                         | Skill discovery, loading, prompt formatting             |
| `src/sse_debug_tests.rs`                      | SSE debug utilities                                     |
| `src/tools/activate_skill_tests.rs`           | activate_skill tool                                     |
| `src/tools/bash_tests.rs`                     | Bash tool execution, truncation                         |
| `src/tools/edit_diff_tests.rs`                | Diff generation for edit tool                           |
| `src/tools/file_queue_tests.rs`               | File queue operations                                   |
| `src/tools/read_tests.rs`                     | Read tool with offset/limit                             |
| `src/tools/rtk_tests.rs`                      | RTK rewrite integration                                 |
| `src/tools/write_tests.rs`                    | Write tool                                              |
| `src/tui/activity_indicator_tests.rs`         | Activity indicator animation                            |
| `src/tui/app_tests.rs`                        | App state, message helpers, token usage                 |
| `src/tui/autocomplete_tests.rs`               | File and skill autocomplete                             |
| `src/tui/editor/commands_tests.rs`            | Editor commands                                         |
| `src/tui/editor/cursor_tests.rs`              | Cursor movement and wrapping                            |
| `src/tui/editor/discovery_tests.rs`           | File discovery for autocomplete                         |
| `src/tui/editor/wrap_tests.rs`                | Text wrapping                                           |
| `src/tui/hook_display_tests.rs`               | Hook display modes                                      |
| `src/tui/layout_tests.rs`                     | Layout calculations, input area height                  |
| `src/tui/messages/hook_message_tests.rs`      | Hook message rendering                                  |
| `src/tui/messages/system_message_tests.rs`    | System message rendering                                |
| `src/tui/messages/thinking_message_tests.rs`  | Thinking block rendering                                |
| `src/tui/messages/tool_call/mod_tests.rs`     | Tool call variant building                              |
| `src/tui/messages/welcome_screen_tests.rs`    | Welcome screen rendering                                |
| `src/tui/status_bar_tests.rs`                 | Status bar rendering                                    |
| `src/tui/thinking_display_tests.rs`           | Thinking display toggle                                 |
| `src/tui/tracker_tests.rs`                    | Token tracker display                                   |

### Widget Crate Tests

| Test File                 | Coverage                                                                                                                    | Tests |
| ------------------------- | --------------------------------------------------------------------------------------------------------------------------- | ----- |
| `crates/ratatui-widgets/` | Panel overlay, autocomplete popup, command palette, prompt editor, header bar                                               | 41    |
| `crates/ratatui-md/`      | Markdown parsing, rendering (headings, paragraphs, lists, tables, code, blockquotes, task lists, rules), height calculation | 175   |
