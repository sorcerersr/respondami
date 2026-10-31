# Quick Start

Get Respondami running in 5 minutes.

## 1. Start Your LLM Server

Respondami needs a running LLM backend. The simplest option is [llama.cpp](https://github.com/ggerganov/llama.cpp):

```bash
# Download a model (if you don't have one)
# Example: https://huggingface.co/models?library=gguf

# Start the server
llama-server -m /path/to/your/model.gguf -c 32768
```

The server starts on `http://localhost:8080` by default.

## 2. Install Respondami

```bash
git clone https://github.com/your-org/respondami.git
cd respondami
cargo build --release
```

The binary is at `target/release/respondami`.

## 3. Configure

On first run, Respondami creates `~/.config/respondami/config.json` with defaults. Edit it to match your setup:

```json
{
  "provider": {
    "type": "llamacpp",
    "url": "http://localhost:8080"
  }
}
```

The `llamacpp` type autodetects the model, context window, and vision support from the server's `/v1/models` endpoint, so no model block is needed. (For a generic OpenAI-compatible endpoint with an explicit model list, use `"type": "openai"` — see [configuration.md](./configuration.md).)

## 4. Run

```bash
./target/release/respondami
```

## 5. Send Your First Message

- Type in the input area at the bottom
- Press **Enter** to send
- The agent streams its response in real-time

## Key Bindings

### Global (any state)

| Key | Action |
|-----|--------|
| `Ctrl+O` / `Ctrl+/` | Toggle reasoning/thinking visibility |
| `Ctrl+T` | Toggle tool output expand/collapse |
| `Ctrl+D` | Quit application |
| `PgUp` / `PgDn` | Scroll chat up/down (full page) |
| `Mouse wheel` | Scroll chat up/down (3 lines) |

### Idle (input mode)

| Key | Action |
|-----|--------|
| `Enter` | Send message / confirm selection |
| `Shift+Enter` / `Alt+Enter` | Insert newline (multi-line input) |
| `Ctrl+C` / `Ctrl+K` | Clear input buffer |
| `Ctrl+E` | Open prompt editor |
| `Ctrl+G` | Open command palette |
| `@` | Trigger file autocomplete |
| `/` (as first character) | Trigger skill autocomplete |
| `Esc` | Clear input buffer / close autocomplete |
| `↑` / `↓` | Navigate autocomplete list |
| `Tab` | Select autocomplete item |

> **Note:** `Shift+Enter` requires a terminal that supports the [Kitty Keyboard Protocol](https://sw.kovidgoyal.net/kitty/keyboard-protocol/) (kitty, foot, WezTerm, Alacritty). On other terminals, `Alt+Enter` works as a fallback.

## Slash Commands

| Command | Action |
|---------|--------|
| `/new` | Start a new session |
| `/resume` | Load an existing session |
| `/compact` | Manually compact context (the command palette's `compact` entry prompts how many recent turns to keep) |
| `/help` | Show available commands |
| `/model` | Show model info and token usage |

## Status Bar

The bottom bar shows the current state, working directory, model, context usage, and cumulative token counts:

```
 ● │ projects │ gpt-4o │ 12% / 131.1k │ ↑108.1k ↓22.3k
```

### State Indicator

| Symbol | State | Meaning |
|--------|-------|---------|
| `●` | Idle | Waiting for input |
| `◐` | Streaming | Receiving tokens from LLM |
| `◆` | Tool Exec | Running a tool (read/write/edit/bash) |
| `◎` | Compacting | Summarizing conversation history |
| `▸` | Session Select | Picking a session to resume |
| `✦` | InitPopup | AGENTS.md generation prompt |
| `⌘` | CommandPalette | Command palette overlay |
| `✎` | PromptEditor | Prompt editor overlay |

### Token Counters

The `↑` and `↓` segments track cumulative token usage for the session:

- `↑` (input) is the largest prompt size observed in the session — context growth. It estimates the tokens the model actually processes, assuming the provider's prompt cache serves the repeated prefix. The raw sum of all requests would be many times larger, so this is the useful number.
- `↓` (output) is the sum of all generated tokens.
- The counter is restored when you resume a session, so it stays stable across close/reopen.

The **Project Token Statistics** dialog (command palette) shows both the raw sum and the `Est. real input` / `Est. real total` lines, which use the same cache-aware math as the status bar.

**Caveat:** the estimate assumes 100% prefix cache hits. A cold cache — the first request of a session, right after compaction, or resuming on another machine or a restarted server — processes more tokens than the estimate. The raw sum is the upper bound.

## Next Steps

- See [configuration.md](./configuration.md) for all config options
- Read [skills.md](./skills.md) to learn about skills
- Read [hooks.md](./hooks.md) to extend Respondami with hooks
- Check [troubleshooting.md](./troubleshooting.md) if you run into issues
- See [security.md](./security.md) for sandboxing recommendations
