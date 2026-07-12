# Logging

Respondami writes structured log output to a file in the current working directory. All log levels (info, warn, error) are written by default.

## Log File Location

```
<current-working-directory>/.respondami/logs/respondami.log
```

The log file is created automatically on first run. The `.respondami` directory is project-scoped, so each project gets its own log file.

**Example**: If you run respondami from `/home/user/project`, the log is at:

```
/home/user/project/.respondami/logs/respondami.log
```

## Log Levels

Respondami uses the standard `tracing` log levels:

| Level | Description |
|-------|-------------|
| `trace` | Very detailed, low-level debug information |
| `debug` | Debugging details for development |
| `info` | General informational messages (always logged) |
| `warn` | Warnings about potential issues (always logged) |
| `error` | Errors (always logged) |

By default, **info, warn, and error** messages are always logged. You cannot disable these using environment variables — they will always appear in the log file.

To also see **trace** and **debug** messages, you need to set the `RUST_LOG` environment variable.

## Setting the Log Level

Use the `RUST_LOG` environment variable to control the logging verbosity.

### Show all messages including debug

```bash
RUST_LOG=debug respondami
```

This shows `debug`, `info`, `warn`, and `error` messages in the log.

### Show all messages including trace (maximum verbosity)

```bash
RUST_LOG=trace respondami
```

### Default (info, warn, error only)

```bash
respondami
```

Or explicitly:

```bash
RUST_LOG=info respondami
```

### Module-specific filtering

You can target specific modules. For example, to enable debug for the `hooks` module only:

```bash
RUST_LOG=hooks=debug respondami
```

### Important: Minimum level is info

The minimum log level is **info**. Setting `RUST_LOG=error` or `RUST_LOG=warn` will still show info messages. The environment variable can only raise the level, never lower it below info.

## Log Rotation

Log files are automatically rotated daily at midnight. Up to **7 daily files** are kept.

**Example rotation sequence:**
```
.respondami/logs/
├── respondami.log        (current, active)
├── respondami.log.2026-06-26
├── respondami.log.2026-06-25
├── respondami.log.2026-06-24
├── respondami.log.2026-06-23
├── respondami.log.2026-06-22
├── respondami.log.2026-06-21
└── respondami.log.2026-06-20      (oldest kept)
```

When a new day begins:
- The current `respondami.log` is renamed to `respondami.log.YYYY-MM-DD`
- A fresh `respondami.log` is created
- Files older than 7 days are automatically discarded

Rotation is handled automatically — no manual intervention needed.

## What Gets Logged

### Application lifecycle

- Application start
- Config loaded
- Application quit

### Provider and endpoint

- LLM endpoint unreachable (warn)
- Provider not configured (warn)
- Endpoint check results

### Session and data

- Skipping corrupted JSONL lines (warn)
- Session save errors (error)
  - Failed to save assistant message
  - Failed to save tool result
  - Failed to save user message
- Session save/load operations

### Tools

- RTK database directory creation failures (warn)
- Skill diagnostics (info)

### Hooks

- Failed to read skill files for hooks (warn)
- Failed to read hook directories (warn)

### Rendering

- Render/layout errors (error)
- Popup rendering debug info (debug, only in debug builds)

## Viewing Logs

View the log file with any text editor or command:

```bash
# View the tail of the log
tail -f .respondami/logs/respondami.log

# View the entire log
cat .respondami/logs/respondami.log

# Search for warnings
grep "WARN" .respondami/logs/respondami.log
```

The log format includes timestamps, log level, and the message, e.g.:

```
2026-06-27T12:00:00.000000Z  INFO respondami: respondami starting
2026-06-27T12:00:00.100000Z  INFO respondami: Config loaded
2026-06-27T12:00:01.000000Z  WARN respondami: LLM endpoint unreachable: Network error: ...
```

## SSE Debug Capture

For diagnosing provider communication issues (empty responses, garbled output, streaming problems), Respondami can capture the full raw request/response exchange.

### Enabling SSE debug

Use the `RESPONDAMI_SSE_DEBUG` environment variable:

| Value | Output directory |
|-------|------------------|
| Unset or empty | Disabled (zero overhead) |
| `1` | `.respondami/sse-debug/` in CWD |
| `/custom/path` | Specified directory |

```bash
# Use default location
RESPONDAMI_SSE_DEBUG=1 respondami

# Custom output directory
RESPONDAMI_SSE_DEBUG=/tmp/sse-debug respondami
```

### Output format

Each agentic turn gets one file named `turn-{session_id}-{seq:04}.log` containing:

- **Request** — full JSON body sent to the provider
- **Response** — raw SSE bytes from the provider

```
.respondami/sse-debug/
├── turn-abc123-0001.log
├── turn-abc123-0002.log
└── turn-abc123-0003.log
```

Each file has a clear structure:

```
=== TURN START (2026-07-12 14:30:00) session=abc123 ===
--- REQUEST 1 (2026-07-12T14:30:00.123) ---
{... JSON body ...}

--- RESPONSE 1 (2026-07-12T14:30:00.456) ---
data: {... SSE chunks ...}

=== TURN END (2026-07-12 14:30:05) ===
```

Multiple iterations within a single turn (e.g. retries, tool loops) are numbered sequentially.

### Use cases

- Diagnose empty or garbled responses from the provider
- Inspect the exact context sent to the model
- Debug SSE parsing issues
- Capture model output for analysis

See [troubleshooting.md](./troubleshooting.md) for common issues and debugging steps.
