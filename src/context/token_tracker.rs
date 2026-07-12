//! Token rate tracking during streaming.
//!
//! Tracks token counts per turn and session totals for persistence.
//! Provider usage data corrects character estimates at end of stream.

use std::time::Instant;

use crate::session::TokenRateEntry;

/// Tracks token counts during streaming turns.
///
/// Character estimates are corrected by provider usage data at end of stream.
/// Session totals are persisted as `TokenRateEntry` entries.
#[derive(Debug, Clone)]
pub struct TokenRateTracker {
    /// Timestamp of the last token arrival.
    last_timestamp: Option<Instant>,
    /// Cumulative chars since turn start (for delta calculation).
    cumulative_chars: usize,
    /// Cumulative estimated tokens at last rate update.
    last_token_estimate: u32,
    /// Turn token count (corrected by provider usage when available).
    turn_tokens: u32,
    /// Inter-token time accumulated for the current turn.
    /// Only counts time between token arrivals (excludes idle/tool gaps).
    turn_stream_seconds: f64,
    /// Whether the first token interval has been processed.
    /// First gap includes prompt processing time — skip it.
    first_interval_done: bool,
    /// Whether actively streaming tokens.
    is_streaming: bool,
    session_tokens: u32,
    session_seconds: f64,
    /// Actual completion tokens from the last provider Usage event.
    provider_completion_tokens: u32,
    /// Whether we have received provider usage data for the current turn.
    has_provider_usage: bool,
}

impl Default for TokenRateTracker {
    fn default() -> Self {
        Self::new()
    }
}

impl TokenRateTracker {
    #[must_use]
    pub fn new() -> Self {
        Self {
            last_timestamp: None,
            cumulative_chars: 0,
            last_token_estimate: 0,
            turn_tokens: 0,
            turn_stream_seconds: 0.0,
            first_interval_done: false,
            is_streaming: false,
            session_tokens: 0,
            session_seconds: 0.0,
            provider_completion_tokens: 0,
            has_provider_usage: false,
        }
    }

    /// Restore session totals from saved entries.
    #[must_use]
    pub fn restore(session_tokens: u32, session_seconds: f64) -> Self {
        Self {
            session_tokens,
            session_seconds,
            ..Self::new()
        }
    }

    /// Called when streaming starts. Resets turn counters.
    pub fn start(&mut self) {
        self.last_timestamp = None;
        self.cumulative_chars = 0;
        self.last_token_estimate = 0;
        self.turn_tokens = 0;
        self.turn_stream_seconds = 0.0;
        self.first_interval_done = false;
        self.is_streaming = true;
        self.provider_completion_tokens = 0;
        self.has_provider_usage = false;
    }

    /// Called on tool result. Resets timestamp to break the measurement
    /// chain so inter-token time doesn't span across tool execution gaps.
    pub fn pause_timing(&mut self) {
        self.last_timestamp = None;
        self.first_interval_done = false;
    }

    /// Called per token arrival. `char_len` is the UTF-8 byte length of the token.
    ///
    /// Accumulates `turn_stream_seconds` (inter-token time only) for the
    /// session average. This excludes idle gaps and tool execution time.
    /// Skips first interval (includes prompt processing time, not generation time).
    pub fn record(&mut self, char_len: usize) {
        let now = Instant::now();
        self.cumulative_chars += char_len;

        // Estimate cumulative tokens from total chars.
        let token_estimate = ((self.cumulative_chars as f64 / 3.5).round() as u32).max(1);

        if let Some(last) = self.last_timestamp {
            if self.first_interval_done {
                let dt = last.elapsed().as_secs_f64();
                // Accumulate inter-token time for session average
                self.turn_stream_seconds += dt;
            } else {
                // First interval — includes prompt processing time. Skip it.
                self.first_interval_done = true;
            }
        }
        // Always update timestamp and estimate.
        self.last_timestamp = Some(now);
        self.last_token_estimate = token_estimate;
        // Keep turn_tokens in sync (will be corrected by provider usage later).
        if token_estimate > self.turn_tokens {
            self.turn_tokens = token_estimate;
        }
    }

    /// Apply actual token counts from a provider Usage event.
    ///
    /// Called when `AgentEvent::Usage` arrives (typically at end of each
    /// streaming phase). Accumulates provider-corrected tokens across phases
    /// so that multi-phase turns (LLM → tools → LLM → …) count all tokens.
    /// The accumulated `provider_completion_tokens` is used in `finalize_turn()`.
    pub fn apply_provider_usage(&mut self, completion_tokens: u32) {
        // Accumulate across phases (was: overwrite with `=`)
        self.provider_completion_tokens += completion_tokens;
        self.has_provider_usage = true;

        // Override the estimated turn tokens with actual provider data
        self.turn_tokens = completion_tokens;
    }

    /// Called when streaming ends. Adds turn totals to session totals.
    /// Returns snapshot to save to session if there was meaningful data.
    /// Prefers provider-corrected tokens and inter-token streaming time.
    pub fn finalize_turn(&mut self) -> Option<TokenRateEntry> {
        self.is_streaming = false;
        // Use provider-corrected tokens if available, otherwise estimated
        let tokens = if self.has_provider_usage {
            self.provider_completion_tokens
        } else {
            self.turn_tokens
        };
        // Use inter-token streaming time for the session average
        let seconds = self.turn_stream_seconds.max(0.01);

        // Always accumulate session totals — even 0-token turns (tool calls only)
        // contribute streaming time.
        self.session_tokens += tokens;
        self.session_seconds += seconds;

        // Reset turn-level counters.
        self.turn_tokens = 0;
        self.turn_stream_seconds = 0.0;
        self.cumulative_chars = 0;
        self.last_token_estimate = 0;
        self.provider_completion_tokens = 0;
        self.has_provider_usage = false;

        (tokens > 0).then(|| TokenRateEntry::new(tokens, seconds))
    }

    /// Whether actively streaming tokens.
    #[must_use]
    pub fn is_streaming(&self) -> bool {
        self.is_streaming
    }
}
