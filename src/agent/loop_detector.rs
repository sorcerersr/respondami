//! Degenerate loop detection for the agent tool-call stream.
//!
//! A *degenerate loop* is the model repeating the exact same tool call
//! (name + arguments) over and over — often with byte-identical thinking and
//! visible text — instead of making progress. In long unattended sessions
//! this can burn minutes and tens of thousands of tokens before a human
//! notices (see the `55b9a55e` session: ~100 identical `grep` calls).
//!
//! [`LoopDetector`] keeps a sliding window of the most recent tool-call keys
//! and reports a [`LoopReport`] when one key occurs at or above a threshold.
//!
//! Frequency-based detection (rather than consecutive-run) is deliberate:
//! real loops interleave "housekeeping" calls (e.g. `edit` to a planning
//! file) between the repeated calls, which defeats a consecutive-run check.
//! Requiring the *exact* same call `threshold` times within `window` calls
//! keeps the false-positive rate negligible — legitimate work almost never
//! issues the identical call that many times in a row.
//!
//! The detector is a pure in-memory observer: it records calls and answers
//! "is this a loop?". The recovery decision (cut, restart, budget) is made
//! by the TUI, which owns the session store.

use serde_json::Value;

/// Default sliding-window size: how many recent tool calls to scan.
///
/// A loop trips within ~`threshold` calls, so the window only needs to be a
/// few times the threshold to catch it; a larger window would just delay the
/// trip and (marginally) raise false-positive risk.
pub const DEFAULT_WINDOW: usize = 20;

/// Default repetition threshold: same (name, args) this many times in the
/// window → loop.
///
/// High enough that legitimate work (which varies its calls) never reaches
/// it, low enough that a true loop trips within minutes.
pub const DEFAULT_THRESHOLD: usize = 8;

/// Default sliding-window size for the text-echo detector: how many recent
/// text-only responses to scan.
pub const DEFAULT_TEXT_WINDOW: usize = 10;

/// Default repetition threshold for the text-echo detector: same normalized
/// text-only response this many times in the window → loop.
///
/// Lower than the tool-call threshold: a text-only turn only repeats when an
/// auto-restart (Stop hook, compaction) re-prompts the model, so three
/// identical final responses are already a near-certain loop.
pub const DEFAULT_TEXT_THRESHOLD: usize = 3;

/// Which degenerate pattern a [`LoopReport`] describes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LoopKind {
    /// Repeated identical tool call (name + arguments).
    ToolCall,
    /// Repeated identical text-only response (no tool calls in the turn).
    TextEcho,
}

/// A detected degenerate loop: one tool call or one text-only response
/// repeated at the threshold.
///
/// Carries everything the TUI needs to locate the cut point (the repeated
/// `tool_name` + canonical `args`, or the repeated text) and to render the
/// recovery message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoopReport {
    /// Which pattern was detected.
    pub kind: LoopKind,
    /// Name of the repeated tool (e.g. `"bash"`, `"read"`); `"response"`
    /// for a text-echo loop.
    pub tool_name: String,
    /// Canonical (sorted-key) JSON of the repeated arguments, or the
    /// normalized repeated text for a text-echo loop.
    pub canonical_args: String,
    /// A short, human-readable preview of the arguments (or text) for display.
    pub args_preview: String,
    /// How many times the call (or response) occurred within the window.
    pub count: usize,
}

impl LoopReport {
    /// A one-line summary for logs and the TUI (e.g. `bash "grep -n foo"`
    /// or `repeated response "I need to investigate…"`).
    #[must_use]
    pub fn summary(&self) -> String {
        match self.kind {
            LoopKind::ToolCall => format!("{} \"{}\"", self.tool_name, self.args_preview),
            LoopKind::TextEcho => format!("repeated response \"{}\"", self.args_preview),
        }
    }
}

/// Sliding-window detector for repeated identical tool calls.
#[derive(Debug)]
pub struct LoopDetector {
    /// Recent tool-call keys, oldest first. Bounded to `window_size`.
    window: Vec<(String, String)>,
    /// Maximum number of recent calls to keep.
    window_size: usize,
    /// Occurrences of a single key at/above which a loop is reported.
    threshold: usize,
}

/// Maximum length of the argument preview in a [`LoopReport`].
///
/// Long commands (multi-line bash) would dominate the TUI message; the
/// preview is for human recognition, not fidelity.
const ARGS_PREVIEW_MAX: usize = 80;

impl LoopDetector {
    /// Create a detector with the given window size and threshold.
    ///
    /// A `window_size` of 0 is treated as 1 (a window must hold at least one
    /// call); a `threshold` of 0 is treated as 1 (every call is a loop).
    #[must_use]
    pub fn new(window_size: usize, threshold: usize) -> Self {
        Self {
            window: Vec::new(),
            window_size: window_size.max(1),
            threshold: threshold.max(1),
        }
    }

    /// Record a tool call, evicting the oldest once the window is full.
    ///
    /// The key is `(tool_name, canonical_args)`; argument key order is
    /// normalized so `{a:1,b:2}` and `{b:2,a:1}` are the same call.
    pub fn record(&mut self, tool_name: &str, args: &Value) {
        let key = (tool_name.to_string(), canonical_args(args));
        self.window.push(key);
        if self.window.len() > self.window_size {
            self.window.remove(0);
        }
    }

    /// Scan the window and report a loop if any key reached the threshold.
    ///
    /// Returns the report for the *most recently completed* offending key
    /// (the one whose latest occurrence is newest), which is the loop the
    /// model is in right now.
    #[must_use]
    pub fn detect(&self) -> Option<LoopReport> {
        // Count occurrences per key and remember the newest index at which
        // each key first reached the threshold.
        let mut best: Option<(usize, String, String, usize)> = None; // (newest_idx, name, args, count)
        for (name, args) in &self.window {
            let count = self
                .window
                .iter()
                .filter(|(n, a)| n == name && a == args)
                .count();
            if count >= self.threshold {
                // The newest occurrence index for this key:
                let newest = self
                    .window
                    .iter()
                    .rposition(|(n, a)| n == name && a == args)
                    .expect("key is in the window");
                // Prefer the key whose newest occurrence is latest (the active loop).
                if best.as_ref().is_none_or(|(bi, _, _, _)| newest > *bi) {
                    best = Some((newest, name.clone(), args.clone(), count));
                }
            }
        }
        best.map(|(_, name, args, count)| {
            let args_preview = preview_args(&args);
            LoopReport {
                kind: LoopKind::ToolCall,
                tool_name: name,
                canonical_args: args,
                args_preview,
                count,
            }
        })
    }
}

/// Sliding-window detector for repeated identical text-only responses.
///
/// A *text-echo loop* is the model ending a turn with the exact same visible
/// text (no tool calls) over and over — typically a Stop-hook re-prompt cycle
/// where the model cannot satisfy the hook and just repeats itself. The loop
/// spans agent re-spawns (each re-prompt restarts the agent), so the detector
/// lives at the TUI level, which sees every `Done`.
///
/// Frequency-based detection within the window (rather than consecutive-run)
/// is deliberate: a loop can interleave distinct "housekeeping" responses
/// between repeats. A turn that executed at least one tool call is progress
/// and clears the window ([`Self::note_tool_activity`]).
#[derive(Debug)]
pub struct TextLoopDetector {
    /// Recent text-only response keys (normalized text), oldest first.
    /// Bounded to `window_size`.
    window: Vec<String>,
    /// Maximum number of recent responses to keep.
    window_size: usize,
    /// Occurrences of a single key at/above which a loop is reported.
    threshold: usize,
}

impl TextLoopDetector {
    /// Create a detector with the given window size and threshold.
    ///
    /// A `window_size` of 0 is treated as 1; a `threshold` of 0 is treated
    /// as 1 (every response is a loop).
    #[must_use]
    pub fn new(window_size: usize, threshold: usize) -> Self {
        Self {
            window: Vec::new(),
            window_size: window_size.max(1),
            threshold: threshold.max(1),
        }
    }

    /// Record a text-only response's text and report a loop if a key reached
    /// the threshold.
    ///
    /// The text is normalized internally (trim + whitespace collapse), so
    /// callers may pass raw response text. Empty (or whitespace-only)
    /// text is not recorded — there is nothing to loop on. The window evicts
    /// the oldest key once full.
    #[must_use]
    pub fn record(&mut self, text: &str) -> Option<LoopReport> {
        let normalized = normalize_text(text);
        if normalized.is_empty() {
            return None;
        }
        self.window.push(normalized);
        if self.window.len() > self.window_size {
            self.window.remove(0);
        }
        self.detect()
    }

    /// A turn that executed at least one tool call is progress: clear the
    /// window so any text-repetition chain is broken.
    pub fn note_tool_activity(&mut self) {
        self.window.clear();
    }

    /// Clear the window (new session, session resume, new user task).
    pub fn reset(&mut self) {
        self.window.clear();
    }

    /// Scan the window and report a loop if any key reached the threshold.
    ///
    /// Returns the report for the *most recently completed* offending key
    /// (the one whose latest occurrence is newest), which is the loop the
    /// model is in right now.
    #[must_use]
    pub fn detect(&self) -> Option<LoopReport> {
        let mut best: Option<(usize, String, usize)> = None; // (newest_idx, key, count)
        for key in &self.window {
            let count = self
                .window
                .iter()
                .filter(|k| k.as_str() == key.as_str())
                .count();
            if count >= self.threshold {
                let newest = self
                    .window
                    .iter()
                    .rposition(|k| k.as_str() == key.as_str())
                    .expect("key is in the window");
                if best.as_ref().is_none_or(|(bi, _, _)| newest > *bi) {
                    best = Some((newest, key.clone(), count));
                }
            }
        }
        best.map(|(_, key, count)| {
            let args_preview = preview_args(&key);
            LoopReport {
                kind: LoopKind::TextEcho,
                tool_name: "response".to_string(),
                canonical_args: key,
                args_preview,
                count,
            }
        })
    }
}

/// Trim and collapse whitespace runs to a single-space key.
///
/// `split_whitespace`/`join` handles leading/trailing whitespace and
/// multi-line text in one pass, so `"a\n\n b"` and `"a b"` are the same key.
#[must_use]
pub fn normalize_text(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Canonical form of a tool call's arguments: JSON with sorted object keys.
///
/// `serde_json::Map` is a `BTreeMap` (this crate does not enable
/// `preserve_order`), so `to_string()` already emits keys in sorted order —
/// the canonical form is the serialization itself. Non-object values
/// (arrays, scalars) serialize deterministically as-is.
#[must_use]
pub fn canonical_args(args: &Value) -> String {
    args.to_string()
}

/// A short, single-line preview of canonical arguments for display.
///
/// Newlines are collapsed (multi-line bash commands) and the result is
/// truncated to [`ARGS_PREVIEW_MAX`] chars with an ellipsis.
#[must_use]
pub fn preview_args(canonical: &str) -> String {
    let flat = canonical.replace(['\n', '\r'], " ");
    if flat.chars().count() <= ARGS_PREVIEW_MAX {
        flat
    } else {
        let truncated: String = flat.chars().take(ARGS_PREVIEW_MAX).collect();
        format!("{truncated}…")
    }
}

/// Build the re-orienting recovery prompt injected as a user message after a
/// loop cut.
///
/// The model has just been cut out of a degenerate loop (the repeated work is
/// gone from its history). The prompt tells it what happened and steers it to
/// re-orient and take a *different* action rather than re-issuing the same
/// call. It names the repeated call and the recovery budget so the model can
/// see this is not its first nudge.
#[must_use]
pub fn loop_recovery_prompt(
    tool_name: &str,
    args_preview: &str,
    count: usize,
    recovery: u32,
    max_recoveries: u32,
) -> String {
    format!(
        "You were repeating the same tool call — {tool_name} \"{args_preview}\" {count} times without making progress, so that repeated work was removed from your history.\n\n\nRe-orient before acting: check the current state of the files and command output you have, then take a DIFFERENT action that moves the task forward. Do not re-issue the identical call. If the task is already done, say so and stop.\n\n(This is loop recovery {recovery} of {max_recoveries}.)"
    )
}

/// Build the recovery prompt for a text-echo loop cut.
///
/// Same structure as [`loop_recovery_prompt`], worded for a repeated
/// *response* rather than a repeated *call*.
#[must_use]
pub fn text_loop_recovery_prompt(
    text_preview: &str,
    count: usize,
    recovery: u32,
    max_recoveries: u32,
) -> String {
    format!(
        "You were repeating the same response — \"{text_preview}\" {count} times without making progress, so that repeated work was removed from your history.\n\n\nRe-orient before acting: check the current state of the files and command output you have, then take a DIFFERENT action that moves the task forward. Do not repeat the identical response. If the task is already done, say so and stop.\n\n(This is loop recovery {recovery} of {max_recoveries}.)"
    )
}
