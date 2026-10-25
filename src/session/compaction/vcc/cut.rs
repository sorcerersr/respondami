//! VCC cut selection — choose where to cut the live window.
//!
//! The live window is the entries after the last compaction (orphan recovery
//! handles a `""` / missing `first_kept_entry_id` sentinel). The cut is anchored
//! to user turns: `keep:N` keeps the last N user turns. When the default
//! `keep:1` tail is small, smart-keep grows it; when the tail is oversized, a
//! token-budget re-cut trims it.
//!
//! Ported from pi-vcc's `before-compact.ts` (`buildOwnCut`, `applyTailBudget`,
//! `resolveSmartKeepUserTurns`), adapted to respondami's entry model. The
//! token budget uses `keep_recent_tokens` (not pi-vcc's fixed 25000).

use crate::agent::token_estimation::{estimate_tokens_from_messages, TokenEstimator};
use crate::session::{build_context_from_entries, AgentMessage, SessionEntry};

/// Smart-keep growth lower bound (tail tokens).
pub const MIN_SMART_TAIL_TOKENS: u32 = 5_000;
/// Smart-keep growth upper bound (tail tokens).
pub const MAX_SMART_TAIL_TOKENS: u32 = 25_000;
/// Tail-oversize factor for the budget re-cut.
pub const OVERSIZED_TAIL_FACTOR: f64 = 2.5;

/// The result of a cut selection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CutResult {
    /// Cut at this entry index (first entry to keep).
    Cut(usize),
    /// Compact everything (no kept tail). `cut_index` is `entries.len()`.
    CompactAll,
    /// Too few live messages to compact.
    TooFew,
}

/// Find the start of the live window (entry index).
///
/// After the last compaction, with orphan recovery: a `""` or missing
/// `first_kept_entry_id` restarts the window right after the compaction entry.
#[must_use]
pub fn find_live_start(entries: &[SessionEntry]) -> usize {
    let Some(last_comp_idx) = entries.iter().rposition(|e| matches!(e, SessionEntry::Compaction { .. }))
    else {
        // No prior compaction: start after the session header.
        return 1;
    };
    let kept_id = match &entries[last_comp_idx] {
        SessionEntry::Compaction {
            first_kept_entry_id, ..
        } => first_kept_entry_id.clone(),
        _ => unreachable!(),
    };
    if kept_id.is_empty() {
        return last_comp_idx + 1;
    }
    match entries.iter().position(|e| e.id() == kept_id) {
        Some(idx) => idx,
        None => last_comp_idx + 1,
    }
}

/// Indices (within the live window) of user turns.
fn user_turn_indices(entries: &[SessionEntry], live_start: usize) -> Vec<usize> {
    (live_start..entries.len())
        .filter(|&i| matches!(&entries[i], SessionEntry::Message { message: AgentMessage::User { .. }, .. }))
        .collect()
}

/// Estimate tokens for an entry slice using the calibrated estimator.
fn estimate_slice_tokens(entries: &[SessionEntry], live_start: usize, cut: usize, est: &TokenEstimator) -> u32 {
    let slice = &entries[live_start..cut];
    let messages = build_context_from_entries(slice);
    estimate_tokens_from_messages(&messages, est)
}

/// Build the own cut for a given `keep` (number of user turns to keep).
///
/// Returns [`CutResult::TooFew`] when the live window has too few messages,
/// [`CutResult::CompactAll`] when the keep request can't form a safe boundary,
/// or [`CutResult::Cut(idx)`] for a valid user-turn-anchored cut.
#[must_use]
pub fn build_own_cut(entries: &[SessionEntry], live_start: usize, keep: usize) -> CutResult {
    let live_len = entries.len() - live_start;
    if live_len <= 2 {
        return CutResult::TooFew;
    }
    if keep == 0 {
        return CutResult::CompactAll;
    }
    let user_idxs = user_turn_indices(entries, live_start);
    let target = user_idxs.len().saturating_sub(keep);
    if target >= user_idxs.len() {
        // keep larger than available user turns.
        return CutResult::CompactAll;
    }
    let cut_idx = user_idxs[target];
    if cut_idx <= live_start {
        return CutResult::CompactAll;
    }
    CutResult::Cut(cut_idx)
}

/// Apply the token-budget tail re-cut.
///
/// - `CompactAll` (from a fallback): re-cut to a `keep_recent_tokens` budget
///   unless the compact-all was explicit (`keep == 0`).
/// - `Cut`: re-cut only when the kept tail exceeds `keep_recent_tokens * 2.5`.
#[must_use]
pub fn apply_tail_budget(
    entries: &[SessionEntry],
    live_start: usize,
    cut: CutResult,
    est: &TokenEstimator,
    keep_recent_tokens: u32,
) -> CutResult {
    match cut {
        CutResult::TooFew => CutResult::TooFew,
        CutResult::CompactAll => {
            let idx = find_budget_cut_index(entries, live_start, keep_recent_tokens, est);
            match idx {
                Some(i) if i > live_start => CutResult::Cut(i),
                _ => CutResult::CompactAll,
            }
        }
        CutResult::Cut(cut_idx) => {
            let tail_tokens = estimate_slice_tokens(entries, cut_idx, entries.len(), est);
            if (tail_tokens as f64) <= (keep_recent_tokens as f64) * OVERSIZED_TAIL_FACTOR {
                return CutResult::Cut(cut_idx);
            }
            let idx = find_budget_cut_index(entries, live_start, keep_recent_tokens, est);
            match idx {
                Some(i) if i > cut_idx => CutResult::Cut(i),
                _ => CutResult::Cut(cut_idx),
            }
        }
    }
}

/// Find a token-budget cut index (nearest valid non-tool boundary).
fn find_budget_cut_index(
    entries: &[SessionEntry],
    live_start: usize,
    max_tokens: u32,
    est: &TokenEstimator,
) -> Option<usize> {
    let mut acc = 0u32;
    let mut crossed: Option<usize> = None;
    for i in (live_start..entries.len()).rev() {
        let tokens = match &entries[i] {
            SessionEntry::Message { .. } => {
                let msgs = build_context_from_entries(&entries[i..=i]);
                estimate_tokens_from_messages(&msgs, est)
            }
            _ => 0,
        };
        acc = acc.saturating_add(tokens);
        if acc >= max_tokens {
            crossed = Some(i);
            break;
        }
    }
    let c = crossed?;
    // Snap forward off any tool result to the next valid boundary.
    entries[c.max(live_start + 1)..]
        .iter()
        .enumerate()
        .find(|(_, e)| !matches!(e, SessionEntry::Message { message: AgentMessage::Tool { .. }, .. }))
        .map(|(off, _)| c.max(live_start + 1) + off)
}

/// Resolve the effective `keep` for smart-keep.
///
/// - `explicit` (user typed `keep:N`) is always respected.
/// - `smart_keep_tail == false` → base keep (1).
/// - Otherwise, if the `keep:1` tail is `<= MIN_SMART_TAIL_TOKENS`, grow keep to
///   the largest N whose tail stays `<= MAX_SMART_TAIL_TOKENS`.
#[must_use]
pub fn resolve_smart_keep(
    entries: &[SessionEntry],
    live_start: usize,
    est: &TokenEstimator,
    smart_keep_tail: bool,
    explicit: bool,
    requested: Option<usize>,
) -> usize {
    let base = requested.unwrap_or(1);
    if explicit || !smart_keep_tail {
        return base;
    }
    let base_cut = build_own_cut(entries, live_start, base);
    let base_tokens = match base_cut {
        CutResult::Cut(idx) => estimate_slice_tokens(entries, idx, entries.len(), est),
        _ => return base,
    };
    if base_tokens > MIN_SMART_TAIL_TOKENS {
        return base;
    }
    let total_user_turns = user_turn_indices(entries, live_start).len();
    let mut selected = base;
    for k in (base + 1)..=total_user_turns {
        let cut = build_own_cut(entries, live_start, k);
        let tokens = match cut {
            CutResult::Cut(idx) => estimate_slice_tokens(entries, idx, entries.len(), est),
            _ => break,
        };
        if tokens > MAX_SMART_TAIL_TOKENS {
            break;
        }
        selected = k;
    }
    selected
}
