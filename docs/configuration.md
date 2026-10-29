# Configuration

Respondami stores its configuration as a JSON file at:

```
$XDG_CONFIG_HOME/respondami/config.json
```

If `$XDG_CONFIG_HOME` is not set, this defaults to `~/.config/respondami/config.json`.

The file is created automatically with default values on first launch. Existing config files are upgraded in place when new sections are added.

## Full Example

```json
{
  "provider": {
    "type": "llamacpp",
    "url": "http://localhost:8080",
    "api_key": "",
    "model": "llama3.2",
    "context_window": 32768,
    "vision": true,
    "strip_images_after_view": false
  },
  "compaction": {
    "enabled": true,
    "mode": "vcc",
    "reserve_tokens": 16384,
    "keep_recent_tokens": 16384,
    "vcc": {
      "smart_keep_tail": true,
      "brief_budget_tokens": 1100
    }
  },
  "ui": {
    "thinking_display": "collapsed",
    "thinking_max_lines": 5,
    "tool_output_expanded": true,
    "hook_display": "minimal"
  },
  "rtk": {
    "enabled": true
  },
  "retry": {
    "enabled": true,
    "max_retries": 3,
    "base_delay_ms": 2000
  }
}
```

## Provider

Settings for connecting to the LLM backend.

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `type` | string | `"llamacpp"` | Provider type. Currently only `"llamacpp"` is supported. |
| `url` | string | `"http://localhost:8080"` | Base URL of the provider server. Must start with `http://` or `https://`. |
| `api_key` | string | `""` | API key for authentication (if required by the server). |
| `model` | string | `"llama3.2"` | Model name to use for completions. |
| `context_window` | integer | `32768` | Context window size in tokens. Must be greater than 0. |
| `vision` | boolean | `true` | Whether this model supports vision (images). When `false`, the `read` tool returns a text note instead of an image attachment. |
| `strip_images_after_view` | boolean | `false` | Workaround for some server/provider environments where images in the conversation interfere with prompt caching. When enabled, an image is removed from the context after the model has seen it (a text note is left in its place, and the file can be re-read). |

## Compaction

Controls automatic conversation history compaction to stay within the context window.

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `enabled` | boolean | `true` | Whether automatic compaction is enabled. |
| `mode` | string | `"vcc"` | Compaction strategy. One of: |
| | | | - `"vcc"` — Algorithmic, LLM-free "vector context checkpoint". Deterministic and instant: extracts a structured summary (session goal, file activity, commits, outstanding context, user preferences) plus a ranked transcript brief. No tokens spent on the compaction itself. |
| | | | - `"llm"` — LLM-based summarization. Sends the conversation to the model and uses its summary. Higher quality for nuanced context, but costs tokens and a model call. |
| `reserve_tokens` | integer | `16384` | Number of tokens to reserve for the next response. Compaction triggers when remaining tokens drop below this value. |
| `keep_recent_tokens` | integer | `16384` | Number of recent tokens to preserve during compaction. Older messages are summarized. |
| `vcc.smart_keep_tail` | boolean | `true` | VCC only. When the default 1-turn kept tail is small (≤ 5000 tokens), grow it to the largest number of recent user turns whose tail stays ≤ 25000 tokens, so short turns don't trigger over-aggressive compaction. |
| `vcc.brief_budget_tokens` | integer | `1100` | VCC only. Token floor for the ranked transcript brief. The actual budget scales with the number of conversation blocks between this floor and a fixed 2000-token ceiling. |

Active skills survive compaction in both modes: their full instructions are re-read from disk and appended to the summary, so the model keeps following them after the `activate_skill` messages are cut.

## UI

Display and interaction settings.

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `thinking_display` | string | `"collapsed"` | How thinking/reasoning blocks are displayed. One of: |
| | | | - `"hidden"` — Shows only `Thinking...`. Keyboard shortcuts have no effect. |
| | | | - `"collapsed"` — Shows `Thinking... ▼`. Press `Ctrl+O` or `Ctrl+/` to expand. |
| | | | - `"expanded"` — Shows header + recent thinking lines. Press `Ctrl+O` or `Ctrl+/` to collapse. |
| `thinking_max_lines` | integer | `5` | Maximum number of thinking lines visible in `expanded` mode. Shows the most recent lines; older lines grow upward out of view. Only applies when `thinking_display` is `"expanded"`. |
| `tool_output_expanded` | boolean | `true` | Default expanded state for tool call output. When `false`, tool calls show a tail view (last N lines). When `true`, tool calls show full output. Toggle with `Ctrl+T`. |
| `hook_display` | string | `"minimal"` | Display mode for hook messages. One of: |
| | | | - `"minimal"` — Shows only hook status (success/blocked). |
| | | | - `"full"` — Shows hook output and timing details. |
| `file_always_visible` | string[] | `["docs/plans/", "docs/reviews/"]` | Gitignore-style patterns that stay visible in the `@` file autocomplete even when gitignored. The config file value replaces the default; `[]` disables. |

## RTK

Controls RTK command rewrite integration. RTK rewrites shell commands before execution, replacing dangerous or inefficient commands with safer alternatives.

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `enabled` | boolean | `true` | Whether RTK rewrite is enabled. RTK must be installed (v0.23.0+) and on `$PATH` for this to take effect. |

## Retry

Controls automatic retry on transient provider errors (empty responses, network failures).

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `enabled` | boolean | `true` | Whether automatic retry is enabled. |
| `max_retries` | integer | `3` | Maximum number of retry attempts before giving up. |
| `base_delay_ms` | integer | `2000` | Base delay between retries in milliseconds. Uses exponential backoff (delay doubles each attempt). |

## Directory Layout

### Skills

Skills are auto-discovered from these locations (no config option). See [skills.md](./skills.md) for details.

| Scope | Path |
|-------|------|
| Global | `~/.config/respondami/skills/<name>/SKILL.md` |
| Project | `.respondami/skills/<name>/SKILL.md` |

Project-level skills override global skills on name collision.

### Hooks

Hooks are loaded from these locations (no config option). See [hooks.md](./hooks.md) for details.

| Scope | Path |
|-------|------|
| Skill frontmatter | `hooks:` block in `SKILL.md` YAML frontmatter |
| Global | `~/.config/respondami/hooks/<event>/<script>.sh` |
| Project | `.respondami/hooks/<event>/<script>.sh` |

Event directories: `UserPromptSubmit`, `PreToolUse`, `PostToolUse`, `PreCompact`, `Stop`.

### AGENTS.md

Project-specific instructions loaded from:

1. `<project-root>/AGENTS.md` — project root (highest priority)
2. `<project-root>/.respondami/AGENTS.md` — hidden project directory (fallback)

Content is appended to the system prompt on every turn.
