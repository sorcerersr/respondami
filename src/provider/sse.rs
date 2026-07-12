//! SSE (Server-Sent Events) stream parsing.
//!
//! Handles buffer management, line splitting, JSON deserialization,
//! and chunk dispatch for OpenAI-compatible SSE streams.
//! Supports cooperative cancellation via a `watch` channel.

use futures_util::StreamExt;
use tokio::sync::mpsc;
use tokio::sync::watch;

use super::{ChatChunk, ProviderError, ToolCallDelta, is_context_overflow};
use crate::session::Usage;
use crate::sse_debug;

// ---------------------------------------------------------------------------
// SSE parsing structs
// ---------------------------------------------------------------------------

/// Ring buffer that keeps the last N raw SSE data lines for diagnostics.
struct RawLineBuffer {
    lines: Vec<String>,
    max_lines: usize,
}

impl RawLineBuffer {
    fn new(max_lines: usize) -> Self {
        Self {
            lines: Vec::with_capacity(max_lines),
            max_lines,
        }
    }

    fn push(&mut self, line: String) {
        if self.lines.len() >= self.max_lines {
            self.lines.remove(0);
        }
        self.lines.push(line);
    }
}

#[derive(serde::Deserialize, Debug)]
pub(super) struct SseChunk {
    #[serde(default)]
    pub(super) choices: Vec<SseChoice>,
    #[serde(default)]
    pub(super) usage: Option<SseUsage>,
}

#[derive(serde::Deserialize, Debug)]
pub(super) struct SseChoice {
    #[serde(default)]
    pub(super) delta: Option<SseDelta>,
    #[serde(default)]
    pub(super) finish_reason: Option<String>,
}

#[derive(serde::Deserialize, Debug)]
pub(super) struct SseDelta {
    #[serde(default)]
    pub(super) content: Option<String>,
    #[serde(default)]
    pub(super) reasoning_content: Option<String>,
    #[serde(default)]
    pub(super) reasoning: Option<String>,
    #[serde(default)]
    pub(super) reasoning_text: Option<String>,
    #[serde(default)]
    pub(super) tool_calls: Option<Vec<async_openai::types::chat::ChatCompletionMessageToolCallChunk>>,
}

#[derive(serde::Deserialize, Debug)]
pub(super) struct SseUsage {
    #[serde(default)]
    pub(super) prompt_tokens: u32,
    #[serde(default)]
    pub(super) completion_tokens: u32,
    #[serde(default)]
    pub(super) total_tokens: u32,
}

// ---------------------------------------------------------------------------
// Parse state tracking
// ---------------------------------------------------------------------------

pub(crate) struct ParseState {
    pub(crate) chunks_received: usize,
    pub(crate) reasoning_received: bool,
    pub(crate) finish_reason_received: bool,
    first_parse_error: Option<String>,
}

impl ParseState {
    pub(crate) fn new() -> Self {
        Self {
            chunks_received: 0,
            reasoning_received: false,
            finish_reason_received: false,
            first_parse_error: None,
        }
    }
}

/// Process a parsed SSE chunk: dispatch reasoning, content, tool-calls, and usage.
///
/// Updates `chunks_received`, `reasoning_received`, and `finish_reason_received`
/// through the mutable `state` reference.
pub(crate) async fn process_sse_chunk(
    sse: SseChunk,
    tx: &mpsc::Sender<ChatChunk>,
    state: &mut ParseState,
) {
    for choice in sse.choices {
        if choice.finish_reason.is_some() {
            state.finish_reason_received = true;
        }

        if let Some(delta) = choice.delta {
            // Thinking / reasoning
            // Use only the FIRST non-empty reasoning field to avoid
            // duplication if the server populates multiple fields
            // (some misconfigured servers send the same content in
            // reasoning_content, reasoning, and reasoning_text).
            let reasoning_text = delta.reasoning_content
                .as_ref()
                .filter(|s| !s.is_empty())
                .or(delta.reasoning.as_ref().filter(|s| !s.is_empty()))
                .or(delta.reasoning_text.as_ref().filter(|s| !s.is_empty()));

            if let Some(text) = reasoning_text {
                state.reasoning_received = true;
                let _ = tx.send(ChatChunk::Thinking(true)).await;
                let _ = tx.send(ChatChunk::Reasoning(text.clone())).await;
            } else {
                let _ = tx.send(ChatChunk::Thinking(false)).await;
            }

            // Content
            if let Some(ref text) = delta.content {
                let _ = tx.send(ChatChunk::Content(text.clone())).await;
                state.chunks_received += 1;
            }

            // Tool calls
            if let Some(ref tool_calls) = delta.tool_calls {
                for tc in tool_calls {
                    let tool_delta = ToolCallDelta {
                        index: tc.index,
                        id: tc.id.clone(),
                        name: tc
                            .function
                            .as_ref()
                            .and_then(|f| f.name.clone()),
                        arguments: tc
                            .function
                            .as_ref()
                            .and_then(|f| f.arguments.clone()),
                    };
                    let _ = tx.send(ChatChunk::ToolCall(tool_delta)).await;
                    state.chunks_received += 1;
                }
            }
        }
    }

    // Usage
    if let Some(usage) = sse.usage {
        let _ = tx.send(ChatChunk::Usage(Usage {
            prompt_tokens: usage.prompt_tokens,
            completion_tokens: usage.completion_tokens,
            total_tokens: usage.total_tokens,
        })).await;
    }
}

/// Record a parse error, capturing only the first one for diagnostics.
fn record_parse_error(state: &mut ParseState, data: &str, label: &str) {
    let preview = data.chars().take(200).collect::<String>();
    tracing::warn!(
        error = %label,
        preview = %preview,
        "SSE data line parse failed"
    );
    if state.first_parse_error.is_none() {
        state.first_parse_error = Some(format!(
            "{} (data: {}...)",
            label,
            preview.chars().take(80).collect::<String>()
        ));
    }
}

// ---------------------------------------------------------------------------
// SSE stream parser
// ---------------------------------------------------------------------------

/// Parse an SSE byte stream and dispatch chunks via the given sender.
///
/// Handles buffer management, line splitting, JSON deserialization,
/// and reasoning/content/tool-call/usage extraction.
///
/// `cancel_rx` — cooperative cancellation channel. When it becomes `true`,
/// the stream is aborted immediately. Returns `Ok(())` on normal completion
/// or on cancellation (the caller can check `cancel_rx` to distinguish).
///
/// `turn_capture` — optional turn-scoped capture. When present, raw response bytes
/// are written to the turn log file. The request portion is written upstream
/// by `stream_chat_inner`.
pub(super) async fn parse_sse_stream(
    response: reqwest::Response,
    tx: mpsc::Sender<ChatChunk>,
    cancel_rx: watch::Receiver<bool>,
    turn_capture: Option<&sse_debug::TurnCaptureRef>,
) -> Result<(), ProviderError> {
    let mut stream = response.bytes_stream();
    let mut buffer = String::new();
    let mut state = ParseState::new();
    let mut raw_buffer = RawLineBuffer::new(20);

    // Write response header before any SSE bytes
    if let Some(capture) = turn_capture {
        capture.write_response_header();
    }

    loop {
        // Check for cancellation before reading the next chunk.
        // This ensures we don't read more data than needed.
        if *cancel_rx.borrow() {
            return Ok(());
        }

        let chunk_result = tokio::select! {
            chunk = stream.next() => chunk,
            () = cancel_rx_changed(&cancel_rx) => {
                // Cancel was triggered while waiting for data
                return Ok(());
            }
        };

        let chunk = match chunk_result {
            Some(Ok(chunk)) => {
                // Write raw bytes to capture file before any parsing
                if let Some(capture) = turn_capture {
                    capture.write_response(&chunk);
                }
                chunk
            }
            Some(Err(e)) => {
                return Err(ProviderError::Network(std::io::Error::other(e)));
            }
            None => {
                // Stream ended normally
                break;
            }
        };

        buffer.push_str(&String::from_utf8_lossy(&chunk));

        while let Some(newline_pos) = buffer.find('\n') {
            // Check cancellation between lines
            if *cancel_rx.borrow() {
                return Ok(());
            }

            let line = buffer[..newline_pos].to_string();
            buffer = buffer[newline_pos + 1..].to_string();

            let line = line.trim_start();
            if line.is_empty() {
                continue;
            }

            if let Some(data) = line.strip_prefix("data: ") {
                raw_buffer.push(data.to_string());
                if data == "[DONE]" {
                    // Don't return — some servers (llama.cpp) send usage after [DONE].
                    // Continue processing remaining events until the stream closes.
                    continue;
                }

                // Check for context overflow error in SSE data line.
                // Some providers send error JSON instead of chat chunks.
                if let Ok(error_obj) = serde_json::from_str::<serde_json::Value>(data)
                    && let Some(message) = error_obj.get("message").and_then(|v| v.as_str())
                    && is_context_overflow(0, message)
                {
                    return Err(ProviderError::ContextOverflow {
                        detail: message.to_string(),
                    });
                }

                match serde_json::from_str::<SseChunk>(data) {
                    Ok(sse) => process_sse_chunk(sse, &tx, &mut state).await,
                    Err(e) => record_parse_error(&mut state, data, &e.to_string()),
                }
            }
        }
    }

    // Process any remaining data in the buffer (last line without trailing newline).
    // Some servers don't terminate the final SSE event with \n.
    // Process ALL data types (content, tool calls, reasoning, usage) — not just usage.
    let remaining = buffer.trim();
    if let Some(data) = remaining.strip_prefix("data: ")
        && data != "[DONE]"
    {
        match serde_json::from_str::<SseChunk>(data) {
            Ok(sse) => process_sse_chunk(sse, &tx, &mut state).await,
            Err(e) => record_parse_error(&mut state, data, &format!("{e} (remaining buffer)")),
        }
    }

    if state.chunks_received == 0 && !state.reasoning_received {
        // Dump raw SSE data lines for diagnostics
        for (i, raw_line) in raw_buffer.lines.iter().enumerate() {
            let preview = raw_line.chars().take(200).collect::<String>();
            tracing::warn!(
                index = i,
                preview = %preview,
                total_bytes = raw_line.len(),
                "SSE empty response — raw data line"
            );
        }

        // Determine error type based on finish_reason presence.
        // If finish_reason was received, the server completed normally but
        // sent no content — treat as EmptyResponse (server-side issue).
        // If no finish_reason, the stream was truncated — treat as
        // IncompleteStream (retryable, matches fantasy's behavior).
        let error = if state.finish_reason_received {
            let detail = match &state.first_parse_error {
                Some(err) => format!("SSE parse failed: {err}"),
                None => "SSE stream ended with no content/tool-call chunks".to_string(),
            };
            ProviderError::EmptyResponse { detail }
        } else {
            let detail = match &state.first_parse_error {
                Some(err) => format!("SSE parse failed: {err}"),
                None => "SSE stream ended with no content/tool-call chunks and no finish_reason".to_string(),
            };
            ProviderError::IncompleteStream { detail }
        };

        tracing::warn!(
            chunks_received = state.chunks_received,
            reasoning_received = state.reasoning_received,
            finish_reason_received = state.finish_reason_received,
            has_parse_errors = state.first_parse_error.is_some(),
            buffer_remaining = buffer.len(),
            "SSE stream ended with no useful chunks"
        );
        return Err(error);
    }

    tracing::debug!(
        chunks_received = state.chunks_received,
        reasoning_received = state.reasoning_received,
        "SSE stream completed"
    );
    Ok(())
}

/// Wait for the `cancel_rx` watch channel to change from its current value.
/// Used in `tokio::select`! to be notified when cancellation is triggered.
async fn cancel_rx_changed(cancel_rx: &watch::Receiver<bool>) {
    let mut rx = cancel_rx.clone();
    let _ = rx.changed().await;
}
