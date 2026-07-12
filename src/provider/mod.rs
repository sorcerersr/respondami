//! Provider abstraction layer.
//!
//! Normalizes provider-specific APIs (OpenAI-compatible, Anthropic, etc.) into
//! a common interface. The agent and TUI depend only on this layer.
//!
//! Rust guideline compliant 2026-02-21

mod llamacpp;
mod sse;

#[doc(inline)]
pub use llamacpp::LlamaCppProvider;

#[cfg(test)]
mod llamacpp_tests;
#[cfg(test)]
mod mod_tests;
#[cfg(test)]
mod sse_tests;

use std::io;

use crate::session::{AgentMessage, ContentBlock, ToolCall, Usage};
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc;

/// Provider-agnostic conversation message.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "role")]
pub enum Message {
    #[serde(rename = "system")]
    System { content: String },
    #[serde(rename = "user")]
    User { content: String },
    #[serde(rename = "assistant")]
    Assistant {
        content: String,
        /// Thinking/reasoning content (sent as `reasoning_content` to API).
        #[serde(default)]
        reasoning: String,
        #[serde(default)]
        tool_calls: Vec<ToolCall>,
    },
    #[serde(rename = "tool")]
    Tool {
        tool_call_id: String,
        result: String,
    },
}

impl Message {
    /// Get the text content of this message.
    #[must_use]
    pub fn content(&self) -> &str {
        match self {
            Message::System { content } | Message::User { content } | Message::Assistant { content, .. } => content,
            Message::Tool { result, .. } => result,
        }
    }
}

/// Request from the agent to a provider.
#[derive(Debug, Clone)]
pub struct ChatRequest {
    pub messages: Vec<Message>,
    pub model: String,
    pub tools: Vec<ToolDef>,
    pub stream: bool,
    pub max_tokens: Option<u32>,
}

/// Tool definition passed to a provider.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolDef {
    pub name: String,
    pub description: String,
    pub parameters: serde_json::Value,
}

/// Normalized chunk yielded during streaming.
#[derive(Debug, Clone)]
pub enum ChatChunk {
    /// Text delta.
    Content(String),
    /// Thinking/reasoning block started (`true`) or ended (`false`).
    Thinking(bool),
    /// Reasoning text delta — hidden by default, counted for t/s.
    Reasoning(String),
    /// Partial tool call being accumulated.
    ToolCall(ToolCallDelta),
    /// Token usage (typically at end of stream).
    Usage(Usage),
}

/// Partial tool call from a streaming delta.
#[derive(Debug, Clone)]
pub struct ToolCallDelta {
    pub index: u32,
    pub id: Option<String>,
    pub name: Option<String>,
    /// JSON string fragment (accumulated across deltas).
    pub arguments: Option<String>,
}

/// Non-streaming completion response.
#[derive(Debug, Clone)]
pub struct CompletionResponse {
    pub content: String,
    pub usage: Option<Usage>,
}

// ---------------------------------------------------------------------------
// Provider trait — extensible abstraction
// ---------------------------------------------------------------------------

/// Core trait that all provider implementations must satisfy.
#[async_trait::async_trait]
pub trait ProviderTrait: Send + Sync {
    /// Clone the underlying provider for use in spawned tasks.
    fn clone_box(&self) -> Box<dyn ProviderTrait>;

    /// Stream a chat completion, sending normalized chunks through `tx`.
    /// The `cancel_rx` watch channel is checked periodically during SSE parsing.
    /// When `cancel_rx` becomes `true`, the SSE stream is aborted.
    async fn stream_chat(
        &self,
        req: &ChatRequest,
        tx: mpsc::Sender<ChatChunk>,
        cancel_rx: tokio::sync::watch::Receiver<bool>,
    ) -> Result<(), ProviderError>;

    /// Non-streaming completion (for compaction).
    async fn complete(&self, req: &ChatRequest) -> Result<CompletionResponse, ProviderError>;

    /// Health check / endpoint reachability.
    async fn ping(&self) -> Result<(), ProviderError>;
}

// ---------------------------------------------------------------------------
// Provider wrapper
// ---------------------------------------------------------------------------

/// Dispatch wrapper for all supported providers.
///
/// Holds a `Box<dyn ProviderTrait>` for dynamic dispatch. Adding a new provider
/// requires only implementing `ProviderTrait` and updating `from_settings()`.
pub struct Provider {
    inner: Box<dyn ProviderTrait>,
}

impl Clone for Provider {
    fn clone(&self) -> Self {
        self.clone_provider()
    }
}

impl std::fmt::Debug for Provider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Provider").finish_non_exhaustive()
    }
}

impl Provider {
    /// Construct a provider from config settings.
    ///
    /// Dispatches to the appropriate provider implementation based on the
    /// `ProviderSettings` variant (currently only `LlamaCpp`).
    ///
    /// # Errors
    ///
    /// Returns [`ProviderError::NotConfigured`] if the provider type is
    /// not configured or unavailable.
    pub fn from_settings(settings: &ProviderSettings) -> Result<Self, ProviderError> {
        match settings {
            ProviderSettings::LlamaCpp(s) => {
                Ok(Provider { inner: Box::new(LlamaCppProvider::from_settings(s)) })
            }
        }
    }

    /// Clone the provider for use in spawned tasks.
    #[must_use]
    pub fn clone_provider(&self) -> Self {
        Provider { inner: self.inner.clone_box() }
    }

    /// Stream a chat completion, sending normalized chunks through `tx`.
    ///
    /// # Errors
    ///
    /// - Network errors if the provider endpoint is unreachable.
    /// - Serialization errors if the response cannot be parsed.
    /// - Context overflow if the request exceeds the model's context window.
    pub async fn stream_chat(
        &self,
        req: &ChatRequest,
        tx: mpsc::Sender<ChatChunk>,
        cancel_rx: tokio::sync::watch::Receiver<bool>,
    ) -> Result<(), ProviderError> {
        self.inner.stream_chat(req, tx, cancel_rx).await
    }

    /// Non-streaming completion (for compaction).
    ///
    /// # Errors
    ///
    /// - Network errors if the provider endpoint is unreachable.
    /// - Serialization errors if the response cannot be parsed.
    pub async fn complete(&self, req: &ChatRequest) -> Result<CompletionResponse, ProviderError> {
        self.inner.complete(req).await
    }

    /// Health check / endpoint reachability.
    ///
    /// # Errors
    ///
    /// Returns an error if the provider endpoint is unreachable.
    pub async fn ping(&self) -> Result<(), ProviderError> {
        self.inner.ping().await
    }
}

// ---------------------------------------------------------------------------
// Config types
// ---------------------------------------------------------------------------

/// Discriminator for provider type.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum ProviderType {
    LlamaCpp,
}

/// Provider-specific settings (extensible for future providers).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ProviderSettings {
    #[serde(rename = "llamacpp")]
    LlamaCpp(LlamaCppSettings),
}

/// Settings for OpenAI-compatible providers (llama.cpp, Ollama, vLLM, `OpenAI`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LlamaCppSettings {
    #[serde(default = "default_url")]
    pub url: String,
    #[serde(default)]
    pub api_key: String,
}

fn default_url() -> String {
    "http://localhost:8080".to_string()
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Convert `AgentMessage` (session domain) to `Message` (provider domain).
#[must_use]
pub fn agent_message_to_message(msg: &AgentMessage) -> Message {
    match msg {
        AgentMessage::System { content } => Message::System {
            content: content.clone(),
        },
        AgentMessage::User { content } => Message::User {
            content: content.clone(),
        },
        AgentMessage::Assistant { content: blocks, .. } => {
            let content = msg.text();
            let reasoning = msg.thinking();
            let tool_calls: Vec<ToolCall> = blocks.iter()
                .filter_map(|b| {
                    if let ContentBlock::ToolCall { tool_call } = b {
                        Some(tool_call.clone())
                    } else {
                        None
                    }
                })
                .collect();
            Message::Assistant {
                content,
                reasoning,
                tool_calls,
            }
        }
        AgentMessage::Tool {
            tool_call_id,
            result,
            ..
        } => Message::Tool {
            tool_call_id: tool_call_id.clone(),
            result: result.clone(),
        },
    }
}

/// Accumulate a `ToolCallDelta` into a `PartialToolCall`.
pub fn accumulate_tool_call(
    pending: &mut Vec<PartialToolCall>,
    delta: &ToolCallDelta,
) {
    let idx = delta.index as usize;
    while pending.len() <= idx {
        pending.push(PartialToolCall {
            id: String::new(),
            name: String::new(),
            arguments: String::new(),
        });
    }
    let call = &mut pending[idx];
    if let Some(ref id) = delta.id {
        call.id = id.clone();
    }
    if let Some(ref name) = delta.name {
        call.name = name.clone();
    }
    if let Some(ref args) = delta.arguments {
        call.arguments.push_str(args);
    }
}

/// Accumulated partial tool call during streaming.
#[derive(Debug, Clone)]
pub struct PartialToolCall {
    pub id: String,
    pub name: String,
    pub arguments: String,
}

/// Parse tool call arguments with lenient JSON repair.
///
/// Modeled on pi-coding-agent's `parseStreamingJson()`: tries strict parse first,
/// then falls back to partial JSON repair. This handles edge cases where the model
/// produces slightly malformed JSON (incomplete braces, trailing commas, unescaped
/// control characters) that would otherwise silently drop the tool call.
///
/// Returns `Some(serde_json::Value)` on success, `None` if all strategies fail.
/// Logs a warning when repair is needed or when parsing ultimately fails.
pub fn parse_tool_call_arguments(raw: &str) -> Option<serde_json::Value> {
    if raw.trim().is_empty() {
        return Some(serde_json::json!({}));
    }

    // Tier 1: strict parse — fast path for well-formed JSON
    if let Ok(value) = serde_json::from_str::<serde_json::Value>(raw) {
        return Some(value);
    }

    // Tier 2: repair incomplete JSON (missing braces, quotes, trailing commas)
    let repaired = partial_json_fixer::fix_json(raw);
    if let Ok(value) = serde_json::from_str::<serde_json::Value>(&repaired) {
        tracing::debug!(
            tool_call_id = %raw.get(..30).unwrap_or(raw),
            original_len = raw.len(),
            repaired_len = repaired.len(),
            "Tool call arguments repaired via partial-json-fixer"
        );
        return Some(value);
    }

    // All strategies failed — log for diagnostics
    tracing::warn!(
        raw_preview = %raw.chars().take(120).collect::<String>(),
        raw_len = raw.len(),
        "Tool call arguments parse failed — dropping tool call"
    );
    None
}

// ---------------------------------------------------------------------------
// Error
// ---------------------------------------------------------------------------

/// Unified error type for all providers.
#[derive(Debug, thiserror::Error)]
pub enum ProviderError {
    #[error("HTTP {status}: {body}")]
    Http {
        status: u16,
        body: String,
        retry_after: Option<std::time::Duration>,
    },

    #[error("Context overflow: {detail}")]
    ContextOverflow { detail: String },

    #[error("Network error: {0}")]
    Network(#[source] io::Error),

    #[error("Serialization error: {0}")]
    Serialization(#[source] serde_json::Error),

    #[error("Provider '{0}' not configured")]
    NotConfigured(String),

    #[error("Provider returned empty response: {detail}")]
    EmptyResponse { detail: String },

    #[error("Incomplete stream: {detail}")]
    IncompleteStream { detail: String },
}

impl ProviderError {
    /// Check if this error is transient and retryable.
    ///
    /// Modeled on pi-coding-agent's `_isRetryableError`:
    /// - Retryable: network errors, rate limits (429), server errors (5xx), timeouts,
    ///   overloaded, serialization failures, empty responses
    /// - Non-retryable: context overflow (handled by compaction), billing/quota errors,
    ///   provider not configured
    #[must_use]
    pub fn is_retryable(&self) -> bool {
        match self {
            // Context overflow → compaction, not retry
            ProviderError::ContextOverflow { .. } => false,

            // Provider not configured → user must fix config
            ProviderError::NotConfigured(_) => false,

            // Network errors are always retryable
            ProviderError::Network(_) => true,

            // Serialization errors may be transient (e.g. truncated response)
            ProviderError::Serialization(_) => true,

            // Empty response is retryable
            ProviderError::EmptyResponse { .. } => true,

            // Incomplete stream is retryable (matches fantasy's IncompleteStreamError)
            ProviderError::IncompleteStream { .. } => true,

            // HTTP errors: classify by status code and body content
            ProviderError::Http { status, body, .. } => {
                // Billing/quota errors are NOT retryable (pi: _isNonRetryableProviderLimitError)
                let body_lower = body.to_lowercase();
                if Self::is_billing_error(&body_lower) {
                    return false;
                }
                // Context overflow via HTTP is NOT retryable
                if is_context_overflow(*status, body) {
                    return false;
                }
                // Rate limits and server errors are retryable
                // 429 (rate limit), 5xx (server errors), 408/409 (timeout/conflict)
                matches!(*status, 408 | 409 | 429 | 500..=599)
            }
        }
    }

    /// Check if error body text indicates a billing/quota limit (non-retryable).
    /// Modeled on pi-coding-agent's `_isNonRetryableProviderLimitError`.
    fn is_billing_error(body_lower: &str) -> bool {
        matches!(
            body_lower,
            s if s.contains("gousagelimiterror")
                || s.contains("freeusagelimiterror")
                || s.contains("monthly usage limit")
                || s.contains("available balance")
                || s.contains("insufficient_quota")
                || s.contains("out of budget")
                || s.contains("quota exceeded")
                || s.contains("billing")
        )
    }
}

/// Check if an HTTP error indicates a context window overflow.
///
/// Strategy (ordered by reliability):
/// 1. Parse JSON body and check `error.type` — stable, versioned identifier.
/// 2. Check HTTP 413 status — standard "Payload Too Large".
/// 3. Substring match on body text — fallback for providers that don't follow
///    the `OpenAI` error schema or use different type values.
#[must_use]
pub fn is_context_overflow(status: u16, body: &str) -> bool {
    // 1. Try to parse structured error and check `error.type`
    if let Ok(val) = serde_json::from_str::<serde_json::Value>(body)
        && let Some(error_type) = val.get("error").and_then(|e| e.get("type")).and_then(|t| t.as_str())
    {
        let context_overflow_types = [
            "exceed_context_size_error",
            "context_length_exceeded",
            "invalid_request_error", // OpenAI uses this for context overflow
        ];
        if context_overflow_types.contains(&error_type) {
            // When we have a structured type, also verify the message mentions
            // context/tokens to avoid false positives on other invalid_request errors.
            if error_type == "invalid_request_error" {
                let body_lower = body.to_lowercase();
                if has_context_overflow_text(&body_lower) {
                    return true;
                }
            } else {
                return true;
            }
        }
    }

    // 2. HTTP 413 is always context overflow
    if status == 413 {
        return true;
    }

    // 3. Fallback: substring match on body text
    let body_lower = body.to_lowercase();
    has_context_overflow_text(&body_lower)
}

/// Substring patterns that indicate context overflow in error messages.
fn has_context_overflow_text(body_lower: &str) -> bool {
    let overflow_patterns = [
        "context window",
        "max tokens",
        "too long",
        "input is too long",
        "maximum context length",
        "exceeds context",
        "context length",
        "token limit",
        "context size",
    ];
    overflow_patterns.iter().any(|p| body_lower.contains(p))
}
