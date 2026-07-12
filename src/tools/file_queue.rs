//! Per-file serialization queue for mutation operations.
//!
//! Uses a global `DashMap` of per-file `Semaphore`s to serialize writes/edits
//! targeting the same file while allowing parallel operations on different files.
//! Stale entries are cleaned up periodically based on idle timeout.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Instant;

use dashmap::DashMap;
use tokio::sync::Semaphore;

/// Serializes file mutation operations targeting the same file.
/// Operations for different files still run in parallel.
pub(crate) struct FileMutationQueueInner {
    /// Per-file semaphores with last-access timestamps, keyed by canonical path.
    /// Entries idle for more than `CLEANUP_IDLE_TIMEOUT` are removed during sweeps.
    pub(crate) semaphores: DashMap<PathBuf, (Arc<Semaphore>, Instant)>,
    /// Operation counter — triggers a cleanup sweep every `CLEANUP_INTERVAL` ops.
    operation_count: AtomicU64,
}

/// How many operations between cleanup sweeps.
const CLEANUP_INTERVAL: u64 = 100;

/// Entries idle longer than this are removed during cleanup.
const CLEANUP_IDLE_TIMEOUT: std::time::Duration = std::time::Duration::from_mins(5);

/// Global file mutation queue.
pub(crate) static FILE_QUEUE: std::sync::LazyLock<FileMutationQueueInner> =
    std::sync::LazyLock::new(|| FileMutationQueueInner {
        semaphores: DashMap::new(),
        operation_count: AtomicU64::new(0),
    });

/// Execute `operation` with per-file serialization.
///
/// If another operation is already in progress for the same file, this will
/// wait until it completes before proceeding. Different files run in parallel.
pub async fn with_file_queue<F, R>(path: &Path, operation: F) -> anyhow::Result<R>
where
    F: std::future::Future<Output = R>,
{
    // Resolve canonical path for deduplication (async)
    let canonical = tokio::fs::canonicalize(path)
        .await
        .unwrap_or_else(|_| PathBuf::from(path));

    // Periodic cleanup of stale entries
    let now = Instant::now();
    let entry = {
        let count = FILE_QUEUE.operation_count.fetch_add(1, Ordering::Relaxed) + 1;
        if count.is_multiple_of(CLEANUP_INTERVAL) {
            cleanup_idle_entries(now);
        }

        let mut entry = FILE_QUEUE
            .semaphores
            .entry(canonical.clone())
            .or_insert_with(|| (Arc::new(Semaphore::new(1)), now));

        entry.1 = now; // Update last-access timestamp
        entry.0.clone()
    };

    // Acquire the permit (blocks if another operation on this file is in progress)
    let _permit = entry
        .acquire()
        .await
        .map_err(|e| anyhow::anyhow!("Failed to acquire file lock for {path:?}: {e}"))?;

    // Execute the operation
    Ok(operation.await)
}

/// Remove entries that have been idle for longer than `CLEANUP_IDLE_TIMEOUT`.
pub(crate) fn cleanup_idle_entries(now: Instant) {
    let mut to_remove = Vec::new();
    for entry in FILE_QUEUE.semaphores.iter_mut() {
        if now.duration_since(entry.value().1) > CLEANUP_IDLE_TIMEOUT {
            to_remove.push(entry.key().clone());
        }
    }
    for key in to_remove {
        FILE_QUEUE.semaphores.remove(&key);
    }
}
