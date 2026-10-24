# TODO

## Code-review 2026-10-23 deferrals

Deferred from the code-review triage (2026-10-23, `docs/plans/2026-10-23-code-review-triage-design.md`). The 36 "fix now" items are done across 7 commits on `next`; these 7 are deferred by design. (L-32 was already tracked here.)

- **L-11:** `toggle_all_tool_output` repeats near-identical arms — DRY only, no bug; a TUI refactor for its own sake.
- **L-25:** `file_queue` canonicalize fallback keys new files by raw path — rare race, low value to fix now.
- **L-26:** `file_queue` cleanup can free a held permit early — unlikely window, hard to reproduce.
- **L-29:** resumed tool results always render `has_error: false` — needs a JSONL schema change to persist the flag; out of scope for this pass.
- **L-34:** `render_status_bar` spawns `git` every frame — perf not correctness; the cache-invalidation strategy is its own design decision.
- **L-36:** test helpers `make_app`/`make_key` duplicated across 5 files — test-only, no runtime impact.

## Future: VCC real read-file tracking (L2 deferral)

Deferred from the arch-findings triage (2026-10-22, `docs/plans/2026-10-22-arch-findings-triage-design.md`).
`vcc/rank.rs::select_ranked_brief_blocks` keeps a `read_files` set that is a plain
clone of `modified_files`, so the `+6` "read" bonus is always applied on top of the
`+18` "modified" bonus for the same paths. It is dead weight, not a bug: the score
ordering is unchanged by the constant double bonus, and the set exists because the
pi-vcc port tracks reads and modifications separately.

To fix properly, port real read tracking:

- `vcc/normalize.rs` — record `read`/`edit`/`write` tool-call targets into separate
  sets (write/edit → modified, read → read-only), threaded through `normalize_entries`
  output (e.g. a small `VccStats` struct alongside the blocks).
- `vcc/rank.rs` — take the two sets as distinct inputs; drop the `modified_files.clone()`.
- Re-check `rank_tests.rs` expectations: modified blocks currently score 24 (18+6),
  read-only blocks would score 6, so ranked selection order can shift for sessions
  with many read-heavy blocks. That shift is the intended behavior change, but the
  brief output is byte-stable test territory, so update expectations deliberately.

## Future: Image clipboard paste (Ctrl+V)

Deferred from the image/vision support design (2026-10-08). The first cut covers
images via the `read` tool only (the model reads an image file, e.g. `@screenshot.png`),
which is the keystone path. Once the `ImageContent` block type and the provider
serializer exist, add a second ingestion path: paste an image from the platform
clipboard (Ctrl+V) into the prompt.

- `src/lib.rs` — the `Event::Paste(text)` handler currently inserts text only. Detect
  an image on the clipboard (Wayland `wl-paste` / X11 `xclip`), write it to a temp file,
  and attach it as an `ImageContent` on the outgoing user message (or insert the temp
  path and let the model `read` it — the reference's approach, one mechanism not two).
- Reuse the same `processImage` normalization pipeline (MIME sniff, convert, resize,
  byte cap) as the `read` tool so pasted and read images are handled identically.
- TUI: show a marker (e.g. `[image: <name>]`) on the user message so the attachment is
  visible in the chat.

## Future: Full image normalization pipeline (EXIF + BMP conversion)

Deferred from the image/vision support design (2026-10-08). The first cut does
MIME sniff + resize + byte cap only. Add the remaining reference-parity pieces if a
real case comes up:

- **EXIF orientation correction** — parse the TIFF `0x0112` orientation tag (JPEG `FF E1`
  segment / WebP `RIFF`+`EXIF` chunk) and apply the 8 orientations before resize, so the
  model sees the image in its intended orientation. Phone screenshots are the main case.
- **BMP → PNG conversion** — detect `BM` magic bytes and convert to PNG via the `image`
  crate (BMP is not an accepted inline format for the OpenAI API). WSLg clipboard is the
  main source of BMP.
- Both run inside the existing `processImage` pipeline, before the resize step.

## Future: Exhaustive panic-elimination (Option B)

The 2026-10-08 "no panics" fix used the **surgical + safety net** approach (Option A):
fix the confirmed `&result[..4096]` char-boundary bug, fix the realistic I/O `expect`,
and install a global panic hook that logs message + location + backtrace. The remaining
"safe in practice" panic sites were deliberately left as-is.

If we later want to eliminate *every* panic path (Option B), address these:

- `src/provider/llamacpp.rs` — reqwest `.expect("invalid ...")` guards (lines ~71, 220,
  225, 303, 372, 378, 420, 431). Convert to `anyhow::Result` so a malformed request
  surfaces as an error instead of a panic.
- `src/sse_debug.rs` — `Mutex::lock().expect("... lock")` (lines ~74, 164, 180, 189,
  209, 232, 246, 260). Only panics if the lock is poisoned by a prior panic. Consider
  recovering the value on poison (see `src/tui/session_state.rs` for the pattern).
- `src/session/manager.rs:461` — `serde_json::to_value(&entry).expect(...)`. Theoretical
  (serializing a well-formed type), but convert to a logged error for completeness.
- `src/agent_events.rs:73`, `src/lib.rs:172/182`, `src/event_loop.rs:184` —
  `.take().unwrap()`/`.expect()` on `is_some()`-guarded `Option`s. Safe in practice;
  could be restructured to bind the value directly to remove the guard-then-take pattern.

Note: the global panic hook (Option A) is the safety net that makes any residual panic
diagnosable, so these are hardening, not urgent.
