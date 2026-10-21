//! Calibrated token estimation — the single source of truth.
//!
//! [`TokenEstimator`] owns a calibrated `chars_per_token` ratio and is the only
//! place token estimation happens in the app. It replaces the scattered
//! `chars/4` and `chars/3.5` heuristics. The ratio is calibrated from real
//! provider usage (`prompt_tokens` vs. the char count of the context) and
//! clamped to a sane range to reject wild values.
//!
//! The legacy free functions ([`estimate_tokens_from_messages`],
//! [`estimate_tokens_from_streamed_text`]) now take a `&TokenEstimator` so the
//! migration is mechanical.

use crate::session::{build_context_from_entries, AgentMessage, ContentBlock, SessionEntry, ToolCall};

/// Chars per token for a single image payload.
///
/// A normalized image is a fixed cost to the model regardless of byte size;
/// 4800 chars ≈ 1200 tokens at the 4.0 heuristic, matching the typical vision
/// patch budget.
pub const IMAGE_CONTENT_CHARS: usize = 4800;

/// Calibrated chars-per-token ratio.
///
/// Cold start is 4.0 (the classic `len/4` heuristic). Once real usage is
/// available, [`TokenEstimator::calibrate`] refines it from the observed
/// `prompt_tokens` vs. context char count, clamped to `[2.0, 8.0]`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TokenEstimator {
    chars_per_token: f64,
}

impl TokenEstimator {
    /// Cold-start heuristic with no usage data yet (4.0 chars/token).
    #[must_use]
    pub fn heuristic() -> Self {
        Self {
            chars_per_token: 4.0,
        }
    }

    /// Calibrate from real usage: total context chars vs. reported tokens.
    ///
    /// The ratio is clamped to `[2.0, 8.0]` to reject wild values (e.g. a
    /// near-empty context with a large `prompt_tokens`, or a tokenizing quirk
    /// that reports far more tokens than the char count implies).
    #[must_use]
    pub fn calibrate(total_chars: usize, actual_tokens: u32) -> Self {
        if total_chars == 0 || actual_tokens == 0 {
            return Self::heuristic();
        }
        let ratio = (total_chars as f64) / (actual_tokens as f64);
        let clamped = ratio.clamp(2.0, 8.0);
        Self {
            chars_per_token: clamped,
        }
    }

    /// The current chars-per-token ratio.
    #[must_use]
    pub fn chars_per_token(&self) -> f64 {
        self.chars_per_token
    }

    /// Estimate tokens for a string as `ceil(len / ratio)`.
    #[must_use]
    pub fn estimate_tokens(&self, text: &str) -> u32 {
        if text.is_empty() {
            return 0;
        }
        (text.len() as f64 / self.chars_per_token).ceil() as u32
    }

    /// Build an estimator calibrated from a list of context messages.
    ///
    /// Uses the char count of all message content divided by the last real
    /// `prompt_tokens` (most recent assistant `Usage`). Falls back to
    /// [`TokenEstimator::heuristic`] when no usage exists.
    #[must_use]
    pub fn from_messages(messages: &[AgentMessage]) -> Self {
        let total_chars = messages.iter().map(estimate_message_content_chars).sum();
        let last_prompt = messages
            .iter()
            .rev()
            .find_map(|m| {
                if let AgentMessage::Assistant { usage, .. } = m {
                    usage.as_ref().map(|u| u.prompt_tokens)
                } else {
                    None
                }
            });
        match last_prompt {
            Some(tokens) => Self::calibrate(total_chars, tokens),
            None => Self::heuristic(),
        }
    }

    /// Build an estimator calibrated from a flat list of session entries.
    ///
    /// Equivalent to [`TokenEstimator::from_messages`] over
    /// [`build_context_from_entries`]. This is the entry point used by
    /// [`crate::session::SessionStore::token_estimator`].
    #[must_use]
    pub fn from_entries(entries: &[SessionEntry]) -> Self {
        let messages = build_context_from_entries(entries);
        Self::from_messages(&messages)
    }
}

impl Default for TokenEstimator {
    fn default() -> Self {
        Self::heuristic()
    }
}

/// Approximate token count from a string using the len/4 heuristic.
///
/// Retained for callers that have no estimator in scope; prefer
/// [`TokenEstimator::estimate_tokens`] for calibrated estimates.
#[must_use]
pub fn approx_token_count(s: &str) -> u32 {
    TokenEstimator::heuristic().estimate_tokens(s)
}

/// Estimate input tokens from a list of messages sent to the model.
///
/// Walks all messages and sums estimated tokens from their content, role
/// labels, tool call metadata, and image payloads.
#[must_use]
pub fn estimate_tokens_from_messages(messages: &[AgentMessage], est: &TokenEstimator) -> u32 {
    messages.iter().map(|m| estimate_message_tokens(m, est)).sum()
}

/// Estimate output tokens from accumulated streaming text.
///
/// Used during streaming to estimate how many output tokens have been
/// generated so far, based on the text accumulated in `streaming_content`.
#[must_use]
pub fn estimate_tokens_from_streamed_text(text: &str, est: &TokenEstimator) -> u32 {
    est.estimate_tokens(text)
}

/// Estimate tokens for a single `AgentMessage`.
fn estimate_message_tokens(msg: &AgentMessage, est: &TokenEstimator) -> u32 {
    match msg {
        AgentMessage::System { content } => {
            est.estimate_tokens("system") + est.estimate_tokens(content)
        }
        AgentMessage::User { content } => {
            est.estimate_tokens("user") + est.estimate_tokens(content)
        }
        AgentMessage::Assistant { content: blocks, .. } => {
            let mut tokens = est.estimate_tokens("assistant");
            for block in blocks {
                match block {
                    ContentBlock::Text { text } => {
                        tokens += est.estimate_tokens(text);
                    }
                    ContentBlock::Thinking { thinking } => {
                        tokens += est.estimate_tokens(thinking);
                    }
                    ContentBlock::ToolCall { tool_call } => {
                        tokens += estimate_tool_call_tokens(tool_call, est);
                    }
                }
            }
            tokens
        }
        AgentMessage::Tool {
            tool_call_id,
            tool_name,
            tool_arguments,
            result,
            images,
        } => {
            let mut tokens = est.estimate_tokens("tool")
                + est.estimate_tokens(tool_call_id)
                + est.estimate_tokens(tool_name)
                + est.estimate_tokens(&tool_arguments.to_string())
                + est.estimate_tokens(result);
            for img in images {
                tokens += crate::image::estimate_image_tokens(img.width, img.height);
            }
            tokens
        }
    }
}

/// Estimate tokens for a tool call.
fn estimate_tool_call_tokens(tc: &ToolCall, est: &TokenEstimator) -> u32 {
    est.estimate_tokens(&tc.id)
        + est.estimate_tokens(&tc.name)
        + est.estimate_tokens(&tc.arguments.to_string())
}

/// Char count of a single message's content for calibration.
///
/// Mirrors the content that actually reaches the wire: text, thinking, tool
/// call metadata, and a fixed cost per image.
fn estimate_message_content_chars(msg: &AgentMessage) -> usize {
    match msg {
        AgentMessage::System { content } => content.len(),
        AgentMessage::User { content } => content.len(),
        AgentMessage::Assistant { content: blocks, .. } => {
            blocks
                .iter()
                .map(|b| match b {
                    ContentBlock::Text { text } => text.len(),
                    ContentBlock::Thinking { thinking } => thinking.len(),
                    ContentBlock::ToolCall { tool_call } => {
                        tool_call.id.len()
                            + tool_call.name.len()
                            + tool_call.arguments.to_string().len()
                    }
                })
                .sum()
        }
        AgentMessage::Tool {
            tool_call_id,
            tool_name,
            tool_arguments,
            result,
            images,
        } => {
            tool_call_id.len()
                + tool_name.len()
                + tool_arguments.to_string().len()
                + result.len()
                + images.len() * IMAGE_CONTENT_CHARS
        }
    }
}
