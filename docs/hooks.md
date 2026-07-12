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

Skills can define hooks in their `SKILL.md` frontmatter:

```yaml
---
name: my-skill
description: My custom skill
hooks:
  PreToolUse:
    - hooks:
        - type: command
          command: echo "security check"
---
```

## Exit Codes

| Code | Effect |
|------|--------|
| `0` | Success — output injected as context |
| `2` | Blocked — action cancelled |
| Other | Non-blocking error — logged, execution continues |

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

The git status output is injected as context before the agent processes your prompt.

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
