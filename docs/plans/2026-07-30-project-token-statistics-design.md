# Project Token Statistics — Design

**Date:** 2026-07-30
**Status:** Draft

## Overview

Add a new command palette entry ("Project Token Statistics") that opens a modal overlay dialog showing aggregate token usage across all sessions in the project. Base unit is million tokens. No real cost calculations — just enough to estimate cost savings from using a local LLM.

## Statistics

| Metric | Description |
|--------|-------------|
| Sessions | Count of session JSONL files |
| Input tokens (M) | Sum of `prompt_tokens` across all Assistant messages |
| Output tokens (M) | Sum of `completion_tokens` across all Assistant messages |
| Total tokens (M) | Input + output |
| Avg input/session (M) | Input tokens ÷ session count |
| Avg output/session (M) | Output tokens ÷ session count |
| I/O ratio (%) | Percentage split between input and output |

## Data Sources

All session JSONL files in `.respondami/sessions/`. Each `Assistant` message carries a `Usage { prompt_tokens, completion_tokens, total_tokens }` struct. Statistics are computed on-demand — no caching layer.

## Architecture

### New module: `src/tui/token_stats.rs`

- `ProjectTokenStats` struct — holds the computed statistics
- `compute_project_stats(cwd: &Path) -> Option<ProjectTokenStats>` — scans session files, aggregates usage
- `render_token_stats_dialog(frame: &mut Frame, stats: &ProjectTokenStats)` — renders the modal overlay

### Integration points

1. **Command palette** — New entry in `get_palette_commands()` with id `"token_stats"`
2. **Command executor** — New branch in `execute_palette_command()`
3. **Modal state** — New field `token_stats: Option<ProjectTokenStats>` on `ModalState`
4. **App state** — New variant `TokenStatsDialog` on `AppState` enum
5. **Layout** — New render branch in layout for the stats overlay
6. **Key handling** — `Esc` dismisses, transitions back to `Idle`

## Data Flow

1. User opens command palette (`Ctrl+G`), selects "Project Token Statistics"
2. `compute_project_stats()` scans `.respondami/sessions/`, parses each JSONL
3. Result stored on `ModalState`, app state → `TokenStatsDialog`
4. Layout renders centered `PanelOverlay` with statistics
5. `Esc` → dialog closes, returns to `Idle`

## Error Handling

- No sessions directory → dialog shows "No session data found"
- Corrupted JSONL lines → skipped with `tracing::warn` (existing pattern)
- Empty project → dialog shows "No sessions yet"

## Dialog Layout

```
┌──────────────────────────────────────┐
│  Project Token Statistics            │
├──────────────────────────────────────┤
│  Sessions:        12                 │
│                                      │
│  Input tokens:    1.23M              │
│  Output tokens:   0.45M              │
│  Total tokens:    1.68M              │
│                                      │
│  Avg input/sess:  0.10M              │
│  Avg output/sess: 0.04M              │
│                                      │
│  Input/Output ratio: 73% / 27%       │
│                                      │
│  Press Esc to close                  │
└──────────────────────────────────────┘
```

## Testing

- Unit tests for `compute_project_stats()` with temp session files
- Edge cases: empty directory, single session, multiple sessions, corrupted lines
- Follows existing pattern in `src/session/manager_tests.rs`
