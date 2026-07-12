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
        }
    }
}
