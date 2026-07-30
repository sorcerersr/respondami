# Welcome Screen — Dim "Not Found" Text & Remove Init Hint

**Date:** 2026-07-13

## Problem

The welcome screen displays "not found" messages with the same `text_muted` color as actual listed items, making them visually indistinguishable. Additionally, the AGENTS.md message includes an `/init` hint that is no longer desired.

## Changes

### Text changes

| Panel | Before | After |
|---|---|---|
| Context | `No AGENTS.md found — /init to generate one` (text_muted) | `No AGENTS.md found` (text_dim) |
| Skills | `No skills loaded` (text_muted) | `No skills loaded` (text_dim) |
| Hooks | `No hooks configured` (text_muted) | `No hooks configured` (text_dim) |

### Implementation

Pass `dim_color: Color` as a new parameter to the three builder functions in `src/tui/messages/welcome_screen.rs`:

- `build_skills_content(skills, dim_color)`
- `build_context_content(cwd, agents_md_path, dim_color)`
- `build_hooks_content(registry, dim_color)`

Wrap "not found" text with `Span::styled(text, Style::default().fg(dim_color))`. The `FilledHeaderBar` paragraph applies `content_fg` as a base style, but the explicit Span style overrides it.

Callers in `render()` pass `theme.text_dim` (`#656c76`).

Also update inline strings used for width calculations in the two-column and three-column layout paths — remove the `/init to generate one` suffix.

### Files

| File | Changes |
|---|---|
| `src/tui/messages/welcome_screen.rs` | Builder signatures, "not found" styling, string updates |
| `src/tui/messages/welcome_screen_tests.rs` | Update builder call sites, verify new text |
