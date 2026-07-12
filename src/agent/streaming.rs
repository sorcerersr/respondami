//! Agent streaming — connects a provider's SSE stream to tool execution.
//!
//! Manages the full streaming lifecycle: sends the chat request to the provider,
//! consumes `ChatChunk` events (content, reasoning, tool calls, usage), executes
//! tool calls as they arrive, and assembles the final `AgentResponse`.

use crate::config::Config;
use crate::provider::{
    accumulate_tool_call, agent_message_to_message, ChatChunk, ChatRequest, Message,
    PartialToolCall, parse_tool_call_arguments, Provider, ProviderError, ToolDef,
};
use crate::session::{AgentMessage, CompactionSettings, ContentBlock, ToolCall, Usage};
use crate::tools::ToolRegistry;
use crate::tui::AgentEvent;
use tokio::sync::mpsc;
use tokio::sync::oneshot;
use tokio::sync::watch;
use std::time::Duration;

/// Response from the agent.
#[derive(Debug)]
pub struct AgentResponse {
    /// Structured content blocks (text, thinking, tool calls).
    pub content: Vec<ContentBlock>,
    pub usage: Option<Usage>,
}

impl AgentResponse {
    /// Concatenate all text blocks.
    #[must_use]
    pub fn text(&self) -> String {
        self.content.iter()
            .filter_map(|b| {
                if let ContentBlock::Text { text } = b {
                    Some(text.as_str())
                } else {
                    None
                }
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// Concatenate all thinking blocks.
    #[must_use]
    pub fn thinking(&self) -> String {
        self.content.iter()
            .filter_map(|b| {
                if let ContentBlock::Thinking { thinking } = b {
                    Some(thinking.as_str())
                } else {
                    None
                }
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// Extract all tool calls from content blocks.
    #[must_use]
    pub fn tool_calls(&self) -> Vec<&ToolCall> {
        self.content.iter()
            .filter_map(|b| {
                if let ContentBlock::ToolCall { tool_call } = b {
                    Some(tool_call)
                } else {
                    None
                }
            })
            .collect()
    }

    /// Check if this response has any thinking blocks.
    #[must_use]
    pub fn has_thinking(&self) -> bool {
        self.content.iter().any(|b| matches!(b, ContentBlock::Thinking { .. }))
    }

    /// Check if this response has any tool call blocks.
    #[must_use]
    pub fn has_tool_calls(&self) -> bool {
        self.content.iter().any(|b| matches!(b, ContentBlock::ToolCall { .. }))
    }

    /// Check if the response is empty (no text, no tool calls, no thinking).
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.text().trim().is_empty() && !self.has_tool_calls() && !self.has_thinking()
    }
}

/// Build a `Provider` from config.
///
/// # Errors
///
/// Returns [`ProviderError`] if the provider settings cannot be converted
/// to a usable provider instance.
pub fn build_provider(config: &Config) -> Result<Provider, ProviderError> {
    Provider::from_settings(&config.provider.settings)
}

/// Stream response from a list of context messages (owned snapshot).
/// Takes messages directly instead of a `SessionStore`.
///
/// `cancel_rx` — cooperative cancellation channel (checked periodically during streaming).
/// When cancellation is detected, the SSE stream is aborted via the provider's `cancel_rx`.
/// Returns `Err(AbortReason::StreamAborted)` if the streaming was aborted.
///
/// # Errors
///
/// - Provider streaming errors (network, serialization, context overflow).
/// - Returns [`AbortReason::StreamAborted`] if the stream was cancelled.
pub async fn stream_response_from_messages(
    provider: &Provider,
    config: &Config,
    context_messages: &[AgentMessage],
    tx: &mpsc::Sender<AgentEvent>,
    tool_registry: &ToolRegistry,
    cancel_rx: watch::Receiver<bool>,
) -> Result<AgentResponse, anyhow::Error> {
    let (chunk_tx, mut chunk_rx) = mpsc::channel::<ChatChunk>(256);
    let messages: Vec<Message> = context_messages.iter().map(agent_message_to_message).collect();

    let tools: Vec<ToolDef> = tool_registry
        .get_definitions()
        .iter()
        .filter(|def| def.name != crate::tools::HOOK_INSTRUCTION_TOOL)
        .map(|def| ToolDef {
            name: def.name.clone(),
            description: def.description.clone(),
            parameters: def.schema.clone(),
        })
        .collect();

    let request = ChatRequest {
        messages,
        model: config.provider.model.clone(),
        tools,
        stream: true,
        max_tokens: None,
    };

    // Spawn the provider streaming task with error propagation via oneshot.
    // This ensures ProviderError::ContextOverflow is not silently dropped.
    let (stream_err_tx, stream_err_rx) = oneshot::channel();
    let provider_clone = provider.clone_provider();
    let req_clone = request.clone();
    let cancel_rx_clone = cancel_rx.clone();
    let stream_handle = tokio::spawn(async move {
        let result = provider_clone
            .stream_chat(&req_clone, chunk_tx, cancel_rx_clone)
            .await;
        // Send error to main task if stream_chat failed
        let _ = stream_err_tx.send(result);
    });

    let mut blocks: Vec<ContentBlock> = Vec::new();
    let mut pending_tool_calls: Vec<PartialToolCall> = Vec::new();
    let mut last_usage: Option<Usage> = None;
    let mut in_thinking = false;
    let mut cancelled = false;

    loop {
        tokio::select! {
            // Receive next chunk from the provider
            chunk = chunk_rx.recv() => {
                match chunk {
                    Some(chunk) => {
                        match chunk {
                            ChatChunk::Content(text) => {
                                // Append to current Text block or create a new one
                                if let Some(last) = blocks.last_mut()
                                    && let ContentBlock::Text { text: existing } = last
                                {
                                    existing.push_str(&text);
                                } else {
                                    blocks.push(ContentBlock::Text { text: text.clone() });
                                }
                                let _ = tx.send(AgentEvent::Token(text)).await;
                            }
                            ChatChunk::Thinking(start) => {
                                if start && !in_thinking {
                                    in_thinking = true;
                                    // Start a new Thinking block
                                    blocks.push(ContentBlock::Thinking { thinking: String::new() });
                                    let _ = tx.send(AgentEvent::ThinkingStart).await;
                                } else if !start && in_thinking {
                                    in_thinking = false;
                                    let _ = tx.send(AgentEvent::ThinkingEnd).await;
                                }
                            }
                            ChatChunk::Reasoning(text) => {
                                // Append to current Thinking block
                                if let Some(last) = blocks.last_mut()
                                    && let ContentBlock::Thinking { thinking: existing } = last
                                {
                                    existing.push_str(&text);
                                }
                                let _ = tx.send(AgentEvent::Reasoning(text)).await;
                            }
                            ChatChunk::ToolCall(delta) => {
                                accumulate_tool_call(&mut pending_tool_calls, &delta);
                            }
                            ChatChunk::Usage(usage) => {
                                last_usage = Some(usage);
                            }
                        }
                    }
                    None => {
                        // Channel closed — streaming finished normally
                        break;
                    }
                }
            }
            // Periodically check for cooperative cancellation
            () = tokio::time::sleep(Duration::from_millis(100)) => {
                if *cancel_rx.borrow() {
                    // Set the flag, the provider's SSE parser will detect it via its own cancel_rx
                    // and stop reading. We also break our loop.
                    cancelled = true;
                    break;
                }
            }
        }
    }

    // Wait for the streaming task to finish.
    // If we cancelled, the task will finish with Ok(()) because the SSE parser
    // returned Ok(()) on cancellation.
    let _ = stream_handle.await;

    // If we cancelled, return early with StreamAborted.
    if cancelled {
        return Err(anyhow::Error::msg(crate::tui::AbortReason::StreamAborted));
    }

    // Receive the error (if any) from the spawned task
    if let Ok(Err(e)) = stream_err_rx.await {
        return Err(e.into());
    }

    // If we ended while still in thinking, emit ThinkingEnd
    if in_thinking {
        let _ = tx.send(AgentEvent::ThinkingEnd).await;
    }

    // Convert partial tool calls to full tool calls and add as ToolCall blocks.
    // Uses lenient JSON parsing (strict → repair → drop) to avoid silently
    // dropping tool calls with slightly malformed arguments.
    let tool_calls: Vec<ToolCall> = pending_tool_calls
        .into_iter()
        .filter_map(|ptc| {
            let args = parse_tool_call_arguments(&ptc.arguments)?;
            Some(ToolCall {
                id: ptc.id,
                name: ptc.name,
                arguments: args,
            })
        })
        .collect();

    // Add tool calls as content blocks
    for tc in tool_calls {
        blocks.push(ContentBlock::ToolCall { tool_call: tc });
    }

    // Remove empty thinking blocks (model started thinking but produced nothing)
    blocks.retain(|b| {
        if let ContentBlock::Thinking { thinking } = b {
            !thinking.is_empty()
        } else {
            true
        }
    });

    // If the provider didn't send usage data, estimate from content.
    let usage = if let Some(u) = last_usage { u } else {
        let text_content: String = blocks.iter()
            .filter_map(|b| {
                if let ContentBlock::Text { text } = b {
                    Some(text.as_str())
                } else {
                    None
                }
            })
            .collect::<Vec<_>>()
            .join("\n");
        let prompt_tokens = CompactionSettings::estimate_tokens(
            &request.messages.iter().map(super::super::provider::Message::content).collect::<Vec<_>>().join("\n"),
        );
        let completion_tokens = CompactionSettings::estimate_tokens(&text_content);
        let total = prompt_tokens.saturating_add(completion_tokens);
        Usage {
            prompt_tokens,
            completion_tokens,
            total_tokens: total,
        }
    };

    Ok(AgentResponse {
        content: blocks,
        usage: Some(usage),
    })
}
