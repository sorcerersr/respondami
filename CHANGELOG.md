# Changelog

All notable changes to this project are documented here.

## [Unreleased]

## [0.3.0] - 2026-11-01

### Features

- Provider: multi-model support with a `/model` picker — per-model `context_window`/`vision`, legacy single-model configs auto-migrate on load
- Provider: llamacpp model autodetection — loaded/vision state from `/v1/models` resolves the active model at runtime; the old LlamaCpp provider is renamed to the generic OpenAI-compatible one, legacy `type: llamacpp` configs migrate on load; the spurious startup "LLM endpoint unreachable" error is gone (model fetch is the reachability check)
- TUI: model select is now a 3-column table (Loaded/Name/Vision); status bar adds a space between the model icon and name; Esc on the picker no longer posts a system message
- Provider: new `strip_images_after_view` option swaps an image for a removal note in live context and session file after the model has seen it, protecting prefix-cache hits on vLLM

### Bug Fixes

- Hooks: skill hooks now survive compaction and no longer double-fire on re-activation
- TUI: the ↑ token counter now restores from max prompt on resume instead of the raw sum, so it stays stable across close/reopen
- TUI: the statistics dialog now leads with cache-aware numbers; the raw sum is a dimmed secondary reference

### Security

- uuid 1.28.0 → 1.29.0
- ignore 0.4.33 → 0.4.34
- similar 3.2.0 → 3.2.1
- unicode-width 0.2.2 → 0.2.3
- rustls 0.23.45 → 0.23.46

## [0.2.1] - 2026-10-25

### Security

- base64 0.22.1 → 0.23.1
- libc 0.2.186 → 0.2.191
- tokio 1.53.3 → 1.54.1
- pulldown-cmark 0.13.4 → 0.13.5
- unicode-normalization 0.1.25 → 0.1.26

## [0.2.0] - 2026-10-25

### Features

- New VCC compaction strategy: deterministic, LLM-free context compaction, now the default (session)
- Skills can now declare lifecycle hooks in frontmatter, with tool matching and model-context injection (hooks)
- New PostCompact hook event with compaction metadata (hooks)

### Bug Fixes

- Agent: a provider task that panics mid-stream now reports a clear stream error instead of a silently truncated response
- Agent: `activate_skill` with an empty name no longer pollutes the active-skills list
- Hooks: PostToolUse hook instructions now reach the model as a separate message and survive session resume
- Hooks: directory hooks are spawned without a shell
- Provider: multibyte UTF-8 characters split across stream chunk boundaries are no longer corrupted
- Provider: reasoning-only turns are kept and replayed as reasoning
- Provider: interleaved thinking phases now render as separate thinking blocks
- Provider: non-streaming completions with an empty response return a clear error
- Session: compacting the whole session no longer drops the first turn
- Session: compaction no longer leaves an orphaned tool result at the start of the kept context
- Session: session file rewrites are now atomic (crash-safe)
- TUI: cancelled read/write/edit tool cards now consistently show the cancelled marker
- TUI: session load failures are surfaced as a system message
- TUI: the welcome screen no longer writes out of bounds on narrow terminals
- TUI: thinking token display uses calibrated estimates
- TUI: hook instructions resume as hook boxes on session load
- Markdown: task lists render correctly and no longer absorb trailing paragraphs
- Markdown: block heights are measured by display width
- Image: animated GIF frame detection no longer miscounts frames
- Tools: the edit tool re-resolves the base file when the file changed
- Tools: the read tool guards against out-of-range offsets
- Other: an empty root AGENTS.md now falls through to `.respondami/AGENTS.md`
- Other: the project statistics dialog distinguishes empty from read errors

## [0.1.0] - 2026-10-21

- Initial release.
