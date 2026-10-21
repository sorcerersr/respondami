# Hooks

Hooks are shell scripts that execute at lifecycle points during the agent's operation. They allow you to inject context, block actions, or run side effects.

## Hook Events

| Event | When it fires |
|-------|---------------|
| `UserPromptSubmit` | When you send a message |
| `PreToolUse` | Before the agent runs a tool |
| `PostToolUse` | After the agent runs a tool |
| `PreCompact` | Before context compaction |
| `Stop` | When the agent finishes a turn |

## Creating Hooks

### Directory-based hooks

Place executable shell scripts in event-named directories:

```
~/.config/respondami/hooks/PreToolUse/security-check.sh
.respondami/hooks/PostToolUse/log-tool.sh
```

| Scope | Path |
|-------|------|
| Global | `~/.config/respondami/hooks/<event>/<script>.sh` |
| Project | `.respondami/hooks/<event>/<script>.sh` |

Scripts must be executable:

```bash
chmod +x ~/.config/respondami/hooks/PreToolUse/security-check.sh
```

### Skill hooks

Skills can define hooks in their `SKILL.md` frontmatter using the
Claude-Code-compatible `hooks:` shape:

```yaml
---
name: my-skill
description: My custom skill
hooks:
  PreToolUse:
    - matcher: "Write|Edit"   # optional; PreToolUse/PostToolUse only
      hooks:
        - type: command
          command: sh ${SKILL_DIR}/scripts/security-check.sh
  UserPromptSubmit:
    - hooks:
        - type: command
          command: sh ${SKILL_DIR}/scripts/inject-context.sh
---
```

- `<Event>` is one of the five hook events. `type` must be `command`.
- `matcher` is an optional case-insensitive regex over the tool name
  (e.g. `Write|Edit`). A non-matching tool skips the hook entirely (no
  shell spawn, no TUI entry). Omitted, `*`, or empty → fires for every
  tool. A regex that fails to compile is logged and the hook never fires.
- Skill hooks fire **only while the skill is active**. They are registered
  when the skill is activated (via a `/skillname` command or the model's
  `activate_skill` tool call) and re-derived from the skill files on
  session resume, so an active skill's hooks are live again after a
  quit + resume without re-activation. `/new` clears them; `reload_hooks`
  preserves them for the currently active skills.
- Skill hook names are tagged with the skill name (e.g.
  `my-skill:security-check.sh`) for deterministic ordering and TUI display.
- `SKILL_DIR` is set to the skill's directory and `SKILL_NAME` to the skill
  name, so scripts can reference their own `scripts/` folder.
- Directory-based hooks always fire for every tool (no matcher concept) and
  are unaffected by skill activation.

## Exit Codes

| Code | Effect |
|------|--------|
| `0` | Success — output injected as context |
| `2` | Blocked — action cancelled |
| Other | Non-blocking error — logged, execution continues |

## Where stdout reaches the model

Every hook execution produces a TUI `HookMessage` entry (Hidden/Minimal/Full
modes). In addition, successful (exit 0) stdout is appended to the model's
context for these events:

| Event | stdout to model? | Where it lands |
|-------|------------------|----------------|
| `UserPromptSubmit` | Yes | Appended to the user message the model sees (TUI shows the clean text) |
| `PreToolUse` | Yes | Appended to the tool result in model context (TUI shows the clean result) |
| `PostToolUse` | Yes | Injected as a synthetic `hook_instruction` tool call |
| `PreCompact` | Yes | Appended to the compaction summary |
| `Stop` | exit 2 → stderr fed back to the model as a new turn | — |

The appended context is a visible `[hook: <name>]` block. Injections land at
the *tail* of the conversation, so the byte-identical, append-only prefix
required for vLLM prefix caching is preserved. A blocked hook (exit 2) injects
nothing.

## Environment Variables

Hooks receive these environment variables:

| Variable | Available in | Description |
|----------|--------------|-------------|
| `HOOK_EVENT` | All | The hook event name (e.g. `PreToolUse`) |
| `HOOK_NAME` | All | The script name |
| `CWD` | All | Current working directory |
| `TOOL_NAME` | `PreToolUse`, `PostToolUse` | Name of the tool being used |
| `TOOL_INPUT` | `PreToolUse`, `PostToolUse` | Tool input as JSON |
| `TOOL_RESULT` | `PostToolUse` | Tool result as JSON |
| `PROMPT` | `UserPromptSubmit` | The user's prompt text |
| `SKILL_NAME` | Skill hooks | Name of the skill defining the hook |
| `SKILL_DIR` | Skill hooks | Directory containing the skill's SKILL.md |

## Display Modes

Control how hook messages appear in the chat via `hook_display` in `config.json`:

| Mode | Display |
|------|---------|
| `minimal` (default) | Shows only hook status (success/blocked) |
| `full` | Shows hook output and timing details |

```json
{
  "ui": {
    "hook_display": "minimal"
  }
}
```

## Examples

### Context injection — inject git status on prompt submit

```bash
#!/bin/bash
# ~/.config/respondami/hooks/UserPromptSubmit/git-status.sh

git status --short 2>/dev/null
exit 0
```

The git status output is appended to the user message the model sees (the
TUI shows only your original prompt).

### Safety guard — block dangerous bash commands

```bash
#!/bin/bash
# ~/.config/respondami/hooks/PreToolUse/no-unsafe-commands.sh

if [ "$TOOL_NAME" = "bash" ]; then
  if echo "$TOOL_INPUT" | grep -qiE '(rm -rf /|mkfs|dd if=)'; then
    exit 2
  fi
fi
exit 0
```

Blocks `rm -rf /`, `mkfs`, and `dd` commands by returning exit code 2.

### Logging — log tool usage to a file

```bash
#!/bin/bash
# ~/.config/respondami/hooks/PostToolUse/tool-logger.sh

echo "$(date -Iseconds) $TOOL_NAME $TOOL_INPUT" >> "$CWD/.respondami/tool-log.txt"
exit 0
```

Logs every tool execution with a timestamp to a file in the project.
