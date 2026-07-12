# Troubleshooting

Common issues and how to fix them.

## Provider Connection Issues

### "LLM endpoint unreachable"

**Cause:** LLM server is not running or URL is incorrect.

**Fix:**

1. Verify your LLM server is running (`llama-server` or equivalent)
2. Check the URL in `~/.config/respondami/config.json` matches your server
3. Test connectivity: `curl http://localhost:8080`

### "Connection refused"

**Cause:** Server is not listening on the expected port, or a firewall is blocking it.

**Fix:**

1. Check the server is bound to the correct interface (not just `127.0.0.1` if accessing remotely)
2. Verify no firewall rules are blocking the port
3. Check `config.json` port matches the server's actual port

## Empty or Garbled Responses

### Deep diagnosis with SSE debug capture

Enable raw request/response logging:

```bash
RESPONDAMI_SSE_DEBUG=1 respondami
```

This writes per-turn files to `.respondami/sse-debug/` containing the full request JSON and raw SSE bytes. Inspect the files to see exactly what the provider sent.

## Skills Not Loading

### SKILL.md not found

**Cause:** Incorrect directory structure or file naming.

**Fix:** Ensure the structure is `<skills-dir>/<name>/SKILL.md` (SKILL.md must be uppercase).

### Skill not in autocomplete

**Cause:** Skill directory is not in a discovery path, or SKILL.md is missing.

**Fix:**

1. Verify the skill is in `~/.config/respondami/skills/<name>/` or `.respondami/skills/<name>/`
2. Verify `SKILL.md` exists and has valid YAML frontmatter with `name` and `description`

## Hooks Not Running

### Script not executable

**Cause:** Missing execute permission.

**Fix:** `chmod +x ~/.config/respondami/hooks/<event>/<script>.sh`

### Wrong directory structure

**Cause:** Event folder name doesn't match exactly (case-sensitive).

**Fix:** Use exact event names: `UserPromptSubmit`, `PreToolUse`, `PostToolUse`, `PreCompact`, `Stop`.

### Hook timeout

**Cause:** Script takes too long to execute.

**Fix:** Keep hooks fast — avoid long-running operations. Check logs for timeout messages.

## Display Issues

### Thinking not showing

**Cause:** `thinking_display` is set to `hidden`.

**Fix:** In `config.json`, set `"thinking_display": "collapsed"` or `"expanded"`. Or press `Ctrl+O` / `Ctrl+/` to toggle.

### Shift+Enter not working for newlines

**Cause:** Terminal doesn't support the Kitty Keyboard Protocol.

**Fix:** Use `Alt+Enter` as a fallback. Supported terminals: kitty, foot, WezTerm, Alacritty.

### Ctrl+O not working in tmux

**Cause:** tmux intercepts Ctrl+O for pane rotation.

**Fix:** Use `Ctrl+/` as the alternative keybinding.

## Debugging

### Enable debug logging

```bash
RUST_LOG=debug respondami
```

Check the log file at `.respondami/logs/respondami.log`.

### Enable SSE debug capture

```bash
RESPONDAMI_SSE_DEBUG=1 respondami
```

Writes raw request/response to `.respondami/sse-debug/turn-*.log`. Useful for diagnosing provider communication issues.

### Common log patterns

| Pattern                          | Meaning                          |
| -------------------------------- | -------------------------------- |
| `LLM endpoint unreachable`       | Server not responding            |
| `Skipping corrupted JSONL lines` | Session file has bad entries     |
| `Session save errors`            | Write failed — check permissions |
| `SSE debug capture enabled`      | SSE debug is active              |
