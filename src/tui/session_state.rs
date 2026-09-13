//! Session persistence state and token usage tracking.
//!
//! Owns the [`SessionStore`], per-request and cumulative token usage, and the
//! shared history guard that keeps outgoing request prefixes byte-identical
//! (see [`crate::history_guard`]).
//!
//! Rust guideline compliant 2026-09-13

use std::sync::{Arc, Mutex, PoisonError};

use crate::history_guard::HistoryGuard;
use crate::session::{RequestTokenUsage, SessionStore};

/// Session persistence and token usage tracking.
#[derive(Debug)]
pub struct SessionState {
    pub session_store: SessionStore,
    pub current_request_usage: RequestTokenUsage,
    pub cumulative_usage: RequestTokenUsage,
    /// Session-level token counts for accurate context window percentage.
    /// These track the total prompt and completion tokens consumed across
    /// all API calls in the session, representing actual context window usage.
    pub session_prompt_tokens: u32,
    pub session_completion_tokens: u32,
    /// Prefix-stability guard shared with every agent run (vLLM
    /// prefix-cache stability). Cloned into each `run_agent_with_snapshot`
    /// spawn; reset on new session, session resume, and compaction.
    pub history_guard: Arc<Mutex<HistoryGuard>>,
}

impl SessionState {
    #[must_use]
    pub fn new(session_store: SessionStore) -> Self {
        Self {
            session_store,
            current_request_usage: RequestTokenUsage::default(),
            cumulative_usage: RequestTokenUsage::default(),
            session_prompt_tokens: 0,
            session_completion_tokens: 0,
            history_guard: Arc::new(Mutex::new(HistoryGuard::new())),
        }
    }

    /// Reset the history guard baseline after a legitimate history rewrite
    /// (new session, session resume, compaction).
    ///
    /// A poisoned lock indicates a prior panic — recover the value anyway;
    /// the guard is log-only and never gates a request.
    pub fn reset_history_guard(&self) {
        self.history_guard
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .reset();
    }
}
