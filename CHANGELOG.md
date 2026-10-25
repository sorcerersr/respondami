# Changelog

All notable changes to this project are documented here.

## [Unreleased]

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
