//! Per-message height cache for the chat viewport.
//!
//! Stores one cached row-height per chat message, keyed by the render
//! parameters (width and display modes). This makes per-frame render cost
//! O(visible) instead of O(total): `height()` is only called for entries that
//! are stale (new or mutated messages), and a stale entry always returns
//! exactly what `height()` produced for the same `(message, key)` — so the
//! rendered output and scroll math are unchanged, only the cost of obtaining
//! it is.
//!
//! The cache is a pure performance optimization. It does no I/O, holds no
//! `Send`/`Sync`-relevant state, and is plain data owned by `ChatState`.
//!
//! Rust guideline compliant 2026-02-21

use crate::tui::hook_display::HookDisplay;
use crate::tui::thinking_display::ThinkingDisplay;

/// Identifies the render parameters a cached height was computed under.
///
/// A mismatch between the current key and the key stored in the cache means
/// every cached height is stale and must be recomputed. This is how terminal
/// resizes and display-mode toggles self-invalidate the cache.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct CacheKey {
    /// Render width in columns.
    pub width: u16,
    /// `ThinkingDisplay` discriminant.
    pub thinking_display: u8,
    /// `thinking_max_lines` config value.
    pub max_lines: u16,
    /// `HookDisplay` discriminant.
    pub hook_display: u8,
}

impl CacheKey {
    /// Build a key from the live render parameters.
    ///
    /// The `usize` fields are narrowed to `u16`/`u8`; terminal widths and
    /// line-count configs never approach those ranges, so no information is
    /// lost in practice.
    #[must_use]
    pub fn new(
        width: usize,
        thinking_display: ThinkingDisplay,
        max_lines: usize,
        hook_display: HookDisplay,
    ) -> Self {
        Self {
            width: width as u16,
            thinking_display: thinking_display as u8,
            max_lines: max_lines as u16,
            hook_display: hook_display as u8,
        }
    }
}

/// A parallel cache of per-message row heights.
///
/// `entries` is kept in lockstep with the chat message list: `entries[i]` is
/// the cached height of message `i`, or `None` when stale. The single `key`
/// records the render parameters the current entries were computed under.
#[derive(Debug, Default)]
pub struct HeightCache {
    key: CacheKey,
    /// Visible to the crate for test assertions on cache length.
    pub(crate) entries: Vec<Option<usize>>,
}

impl HeightCache {
    /// Create an empty cache.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Clear all cached heights.
    ///
    /// Used when the message list is replaced with different content at a
    /// possibly-equal length (e.g. session resume), which the length resync
    /// cannot detect.
    pub fn invalidate_all(&mut self) {
        self.entries.clear();
    }

    /// Mark a single entry stale.
    ///
    /// Used after a content mutation at a known index (e.g. a streaming token
    /// appended to the last message). Out-of-bounds indices are ignored.
    pub fn invalidate(&mut self, index: usize) {
        if let Some(entry) = self.entries.get_mut(index) {
            *entry = None;
        }
    }

    /// Resync the cache length to the message list length.
    ///
    /// Truncates if the list shrank (clear/truncate) and extends with stale
    /// `None` entries if it grew (push). This makes structural changes free to
    /// handle: no explicit invalidation call is needed at push/clear/truncate
    /// sites.
    pub fn resync_len(&mut self, len: usize) {
        if self.entries.len() > len {
            self.entries.truncate(len);
        } else if self.entries.len() < len {
            self.entries.resize(len, None);
        }
    }

    /// Return the cached height for entry `i`, computing and storing it on a miss.
    ///
    /// If `key` differs from the key the entries were computed under, all
    /// entries are cleared first (a resize or display toggle). If `i` is out
    /// of bounds the entry is extended defensively and computed. The `compute`
    /// closure is invoked at most once per call and only on a miss.
    pub fn get_or_compute<F: FnOnce() -> usize>(&mut self, i: usize, key: CacheKey, compute: F) -> usize {
        if key != self.key {
            self.entries.clear();
            self.key = key;
        }
        if i >= self.entries.len() {
            self.entries.resize(i + 1, None);
        }
        if let Some(h) = &self.entries[i] {
            *h
        } else {
            let h = compute();
            self.entries[i] = Some(h);
            h
        }
    }

    /// Sum the heights of all `len` entries, computing any that are stale.
    ///
    /// `compute(i)` is invoked for each stale entry. Used by the render path
    /// to obtain the total content height in a single cached walk.
    pub fn total<F: FnMut(usize) -> usize>(&mut self, len: usize, key: CacheKey, mut compute: F) -> usize {
        if key != self.key {
            self.entries.clear();
            self.key = key;
        }
        self.entries.resize(len, None);
        let mut sum = 0usize;
        for (i, entry) in self.entries.iter_mut().enumerate() {
            if let Some(h) = entry {
                sum += *h;
            } else {
                let h = compute(i);
                *entry = Some(h);
                sum += h;
            }
        }
        sum
    }
}
