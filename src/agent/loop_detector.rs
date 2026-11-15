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

/// A detected degenerate loop: one tool call repeated at the threshold.
///
/// Carries everything the TUI needs to locate the cut point (the repeated
/// `tool_name` + canonical `args`) and to render the recovery message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoopReport {
    /// Name of the repeated tool (e.g. `"bash"`, `"read"`).
    pub tool_name: String,
    /// Canonical (sorted-key) JSON of the repeated arguments.
    pub canonical_args: String,
    /// A short, human-readable preview of the arguments for display.
    pub args_preview: String,
    /// How many times the call occurred within the window.
    pub count: usize,
}

impl LoopReport {
    /// A one-line summary for logs and the TUI (e.g. `bash "grep -n foo"`).
    #[must_use]
    pub fn summary(&self) -> String {
        format!("{} \"{}\"", self.tool_name, self.args_preview)
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
                tool_name: name,
                canonical_args: args,
                args_preview,
                count,
            }
        })
    }
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
