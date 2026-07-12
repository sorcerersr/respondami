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
    "url": "http://localhost:8080",
    "model": "llama3.2",
    "context_window": 32768
  }
}
```

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
| `/compact` | Manually compact context |
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

## Next Steps

- See [configuration.md](./configuration.md) for all config options
- Read [skills.md](./skills.md) to learn about skills
- Read [hooks.md](./hooks.md) to extend Respondami with hooks
- Check [troubleshooting.md](./troubleshooting.md) if you run into issues
- See [security.md](./security.md) for sandboxing recommendations
