# Hooks

Hooks are shell scripts that execute at lifecycle points during the agent's operation. They are pure side-effects + vetoes: they run scripts, block actions, and — for `PreCompact` only — feed the compaction summary. They do not pollute the model's context with instructions.

## Hook Events

| Event | When it fires |
|-------|---------------|
| `UserPromptSubmit` | When you send a message |
| `PreToolUse` | Before the agent runs a tool |
| `PostToolUse` | After the agent runs a tool |
| `PreCompact` | Before context compaction |
| `PostCompact` | After a compaction completes successfully |
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
  PostToolUse:
    - matcher: "Write|Edit"
      hooks:
        - type: command
          command: sh ${SKILL_DIR}/scripts/log-tool.sh
---
```

- `<Event>` is one of the six hook events. `type` must be `command`.
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
| `0` | Success — side effects run; no model-bound output (except `PreCompact`, see below) |
| `2` | Blocked — the action is vetoed; the reason (stderr) is surfaced to the model |
| Other | Non-blocking error — logged, execution continues |

## Where stdout reaches the model

Every hook execution produces a TUI `HookMessage` entry (Hidden/Minimal/Full
modes). Hook stdout is **never** sent to the model, with exactly two
exceptions:

| Event | stdout to model? | Where it lands |
|-------|------------------|----------------|
| `PreCompact` (exit 0) | Yes — the only success-path injection | Appended to the compaction summary |
| `PreToolUse` (exit 2) | Blocked reason (stderr) | Returned as the tool result (`is_error`); the model sees "access denied: <reason>" |
| `PostToolUse` (exit 2) | Blocked reason (stderr) | Appended to the model-bound tool result |
| `Stop` (exit 2) | stderr | Fed back to the model as a new turn |
| `PreCompact` (exit 2) | — (shown to the user as a system message) | Compaction vetoed; the turn proceeds uncompacted |
| `PostCompact` (exit 2) | — (logged as a non-blocking error) | Cannot undo the compaction |
| All other events | No | Side effects only |

A blocked hook (exit 2) injects nothing on the success path; only its block
reason is surfaced. The `PreCompact` success injection lands at the *tail* of
the compaction summary, so the byte-identical, append-only prefix required for
vLLM prefix caching is preserved.

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
| `COMPACT_TRIGGER` | `PostCompact` | `manual` or `auto` |
| `COMPACT_MESSAGES_REMOVED` | `PostCompact` | Number of messages removed by the compaction |
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

### Side effect — log git status on prompt submit

```bash
#!/bin/bash
# ~/.config/respondami/hooks/UserPromptSubmit/git-status.sh

git status --short 2>/dev/null >> "$CWD/.respondami/git-status.log"
exit 0
```

Logs the git status to a file. The output is a side effect only — it is not
sent to the model (only `PreCompact` success stdout is).

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
