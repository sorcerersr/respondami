//! Unit tests for the per-message height cache.
//!
//! Uses counting closures to verify that `height()` is invoked exactly the
//! expected number of times: once per stale entry, never for a fresh cache
//! hit. This is the core invariant that makes the render path O(visible).

use super::height_cache::{CacheKey, HeightCache};
use crate::tui::hook_display::HookDisplay;
use crate::tui::thinking_display::ThinkingDisplay;

/// A key with distinct, fixed render parameters.
fn key() -> CacheKey {
    CacheKey::new(120, ThinkingDisplay::Collapsed, 5, HookDisplay::Minimal)
}

/// A key that differs from [`key`] in width (simulates a terminal resize).
fn other_key() -> CacheKey {
    CacheKey::new(200, ThinkingDisplay::Collapsed, 5, HookDisplay::Minimal)
}

#[test]
fn new_cache_is_empty() {
    let cache = HeightCache::new();
    assert_eq!(cache.entries.len(), 0);
}

#[test]
fn get_or_compute_computes_once_then_hits() {
    let mut cache = HeightCache::new();
    let mut calls = 0usize;
    let k = key();

    let first = cache.get_or_compute(0, k, || {
        calls += 1;
        42
    });
    let second = cache.get_or_compute(0, k, || {
        calls += 1;
        999
    });

    assert_eq!(first, 42);
    assert_eq!(second, 42, "second call must return the cached value");
    assert_eq!(calls, 1, "compute closure must run exactly once");
}

#[test]
fn get_or_compute_per_index_is_independent() {
    let mut cache = HeightCache::new();
    let mut calls = 0usize;
    let k = key();

    cache.get_or_compute(0, k, || {
        calls += 1;
        1
    });
    cache.get_or_compute(1, k, || {
        calls += 1;
        2
    });
    cache.get_or_compute(0, k, || {
        calls += 1;
        999
    });

    assert_eq!(calls, 2, "index 0 hit must not recompute; index 1 computed once");
}

#[test]
fn total_computes_each_stale_entry_once() {
    let mut cache = HeightCache::new();
    let mut calls = 0usize;
    let k = key();

    let total = cache.total(3, k, |i| {
        calls += 1;
        i + 1
    });

    assert_eq!(total, 6, "1 + 2 + 3");
    assert_eq!(calls, 3, "each of the 3 entries computed once");

    // A second total with the same key must be a full cache hit.
    let mut calls2 = 0usize;
    let total2 = cache.total(3, k, |i| {
        calls2 += 1;
        i + 1
    });
    assert_eq!(total2, 6);
    assert_eq!(calls2, 0, "no recomputation on a warm cache");
}

#[test]
fn invalidate_marks_single_entry_stale() {
    let mut cache = HeightCache::new();
    let k = key();

    for i in 0..3 {
        cache.get_or_compute(i, k, || i + 1);
    }
    cache.invalidate(1);

    let mut calls = 0usize;
    let h0 = cache.get_or_compute(0, k, || {
        calls += 1;
        100
    });
    let h1 = cache.get_or_compute(1, k, || {
        calls += 1;
        101
    });
    let h2 = cache.get_or_compute(2, k, || {
        calls += 1;
        102
    });

    assert_eq!(h0, 1);
    assert_eq!(h1, 101, "invalidated entry must recompute");
    assert_eq!(h2, 3);
    assert_eq!(calls, 1, "only the invalidated entry recomputes");
}

#[test]
fn invalidate_out_of_bounds_is_noop() {
    let mut cache = HeightCache::new();
    let k = key();
    cache.get_or_compute(0, k, || 5);

    cache.invalidate(99);

    let h = cache.get_or_compute(0, k, || 999);
    assert_eq!(h, 5, "out-of-bounds invalidate must not affect other entries");
}

#[test]
fn invalidate_all_clears_everything() {
    let mut cache = HeightCache::new();
    let k = key();
    for i in 0..3 {
        cache.get_or_compute(i, k, || i + 1);
    }
    cache.invalidate_all();

    let mut calls = 0usize;
    let total = cache.total(3, k, |i| {
        calls += 1;
        i + 10
    });
    assert_eq!(total, 33, "0+10, 1+10, 2+10");
    assert_eq!(calls, 3, "all entries recomputed after invalidate_all");
}

#[test]
fn resync_len_truncates_when_list_shrinks() {
    let mut cache = HeightCache::new();
    let k = key();
    for i in 0..5 {
        cache.get_or_compute(i, k, || i + 1);
    }
    cache.resync_len(2);
    assert_eq!(cache.entries.len(), 2);

    // The surviving entries are still valid.
    let h = cache.get_or_compute(1, k, || 999);
    assert_eq!(h, 2);
}

#[test]
fn resync_len_extends_with_stale_when_list_grows() {
    let mut cache = HeightCache::new();
    let k = key();
    cache.get_or_compute(0, k, || 1);
    cache.resync_len(4);
    assert_eq!(cache.entries.len(), 4);

    let mut calls = 0usize;
    let total = cache.total(4, k, |i| {
        calls += 1;
        i + 1
    });
    assert_eq!(total, 10, "1+2+3+4");
    assert_eq!(calls, 3, "only the 3 new entries are computed");
}

#[test]
fn key_mismatch_clears_all_entries() {
    let mut cache = HeightCache::new();
    for i in 0..3 {
        cache.get_or_compute(i, key(), || i + 1);
    }

    // A different key (resize) must force full recomputation.
    let mut calls = 0usize;
    let total = cache.total(3, other_key(), |i| {
        calls += 1;
        i + 100
    });
    assert_eq!(total, 303, "100+101+102");
    assert_eq!(calls, 3, "key change invalidates every entry");
}

#[test]
fn get_or_compute_extends_defensively_for_out_of_bounds() {
    let mut cache = HeightCache::new();
    let k = key();
    cache.get_or_compute(0, k, || 1);

    // Index beyond current length is extended and computed.
    let h = cache.get_or_compute(5, k, || 42);
    assert_eq!(h, 42);
    assert_eq!(cache.entries.len(), 6);
}

#[test]
fn total_resizes_to_len_before_summing() {
    let mut cache = HeightCache::new();
    let k = key();
    cache.get_or_compute(0, k, || 1);
    cache.get_or_compute(1, k, || 2);

    // total with a smaller len truncates and sums only the kept entries.
    let total = cache.total(1, k, |i| {
        1000 + i
    });
    assert_eq!(total, 1, "only entry 0 kept");
    assert_eq!(cache.entries.len(), 1);
}
