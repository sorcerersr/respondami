//! Llama.cpp / OpenAI-compatible provider implementation.
//!
//! Maps `OpenAI` Chat Completions API to the normalized provider abstraction.

use async_openai::config::OpenAIConfig;
use async_openai::types::chat::{
    ChatCompletionRequestAssistantMessageArgs,
    ChatCompletionRequestMessage,
    ChatCompletionRequestSystemMessageArgs,
    ChatCompletionRequestToolMessageArgs,
    ChatCompletionRequestUserMessageArgs,
    ChatCompletionMessageToolCall,
    ChatCompletionMessageToolCalls,
    ChatCompletionTool,
    ChatCompletionTools,
    CreateChatCompletionRequest,
    CreateChatCompletionRequestArgs,
    FunctionCall,
    FunctionObject,
};
use async_openai::Client;

use super::{
    ChatChunk, ChatRequest, CompletionResponse, is_context_overflow,
    LlamaCppSettings, Message, ProviderError, ProviderTrait,
};
use crate::session::Usage;
use tokio::sync::mpsc;
use tokio::sync::watch;

/// User-Agent string for HTTP requests to the provider.
const USER_AGENT: &str = concat!("Respondami/", env!("CARGO_PKG_VERSION"));

/// Provider for OpenAI-compatible endpoints (llama.cpp, Ollama, vLLM, `OpenAI`).
pub struct LlamaCppProvider {
    url: String,
    api_key: String,
    client: Client<OpenAIConfig>,
    http_client: reqwest::Client,
}

impl std::fmt::Debug for LlamaCppProvider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LlamaCppProvider")
            .field("url", &self.url)
            .field("api_key", &"[redacted]")
            .finish()
    }
}

impl LlamaCppProvider {
    /// Create a Llama.cpp provider from settings.
    ///
    /// # Panics
    ///
    /// - If `reqwest::Client::builder().build()` fails (extremely rare, only on
    ///   misconfigured HTTP stack).
    #[must_use]
    pub fn from_settings(settings: &LlamaCppSettings) -> Self {
        let mut cfg = OpenAIConfig::new().with_api_base(&settings.url);
        if !settings.api_key.is_empty() {
            cfg = cfg.with_api_key(&settings.api_key);
        }
        Self {
            url: settings.url.clone(),
            api_key: settings.api_key.clone(),
            client: Client::with_config(cfg),
            http_client: reqwest::Client::builder()
                .user_agent(USER_AGENT)
                .build()
                .expect("reqwest client build"),
        }
    }

    /// Clone the provider (rebuilds internal client from settings).
    #[must_use]
    pub fn clone_provider(&self) -> Self {
        Self {
            url: self.url.clone(),
            api_key: self.api_key.clone(),
            client: self.client.clone(),
            http_client: self.http_client.clone(),
        }
    }

    // -----------------------------------------------------------------------
    // Streaming chat
    // -----------------------------------------------------------------------

    pub(super) async fn stream_chat_inner(
        &self,
        req: &ChatRequest,
        tx: mpsc::Sender<ChatChunk>,
        cancel_rx: watch::Receiver<bool>,
    ) -> Result<(), ProviderError> {
        let openai_req = build_openai_request(req, true);
        let request_json =
            serde_json::to_value(&openai_req).map_err(ProviderError::Serialization)?;

        // Inject reasoning_content into assistant messages.
        // async-openai doesn't support this field, so we post-process the JSON.
        let request_json = inject_reasoning_content(request_json, &req.messages);
        let request_body =
            serde_json::to_string(&request_json).map_err(ProviderError::Serialization)?;

        // Write request to turn capture if active
        if let Some(capture) = crate::sse_debug::current_turn() {
            capture.write_request(&request_body);
        }

        // Debug logging: request details for tracing empty response issues
        tracing::debug!(
            messages = req.messages.len(),
            body_bytes = request_body.len(),
            model = %req.model,
            "Sending chat completion request"
        );

        let url = format!("{}/chat/completions", self.url);

        let mut req_builder = self.http_client
            .post(&url)
            .header("Content-Type", "application/json")
            .header("Accept", "text/event-stream")
            .body(request_body);

        if !self.api_key.is_empty() {
            req_builder =
                req_builder.header("Authorization", format!("Bearer {}", self.api_key));
        }

        let response = req_builder
            .send()
            .await
            .map_err(|e| ProviderError::Network(std::io::Error::other(e)))?;

        if !response.status().is_success() {
            let status = response.status().as_u16();

            // Extract Retry-After header for rate limit responses
            let retry_after = response
                .headers()
                .get("Retry-After")
                .and_then(|v| v.to_str().ok())
                .and_then(|s| s.parse::<u64>().ok())
                .map(std::time::Duration::from_secs);

            let body = match response.text().await {
                Ok(text) => text,
                Err(e) => {
                    return Err(ProviderError::Http {
                        status,
                        body: format!("(failed to read body: {e})"),
                        retry_after,
                    });
                }
            };
            if is_context_overflow(status, &body) {
                return Err(ProviderError::ContextOverflow {
                    detail: format!("HTTP {}: {}", status, body.trim()),
                });
            }
            return Err(ProviderError::Http { status, body, retry_after });
        }

        super::sse::parse_sse_stream(response, tx, cancel_rx, crate::sse_debug::current_turn().as_ref()).await
    }

    // -----------------------------------------------------------------------
    // Non-streaming completion (for compaction)
    // -----------------------------------------------------------------------

    pub(super) async fn complete_inner(
        &self,
        req: &ChatRequest,
    ) -> Result<CompletionResponse, ProviderError> {
        let openai_req = build_openai_request(req, false);

        // Strip stream flag for non-streaming
        let request: CreateChatCompletionRequest = serde_json::from_value(
            serde_json::to_value(&openai_req).map_err(ProviderError::Serialization)?,
        )
        .map_err(ProviderError::Serialization)?;

        let response = self
            .client
            .chat()
            .create(request)
            .await
            .map_err(|e| ProviderError::Network(std::io::Error::other(e)))?;

        let content = response
            .choices
            .first()
            .and_then(|c| c.message.content.clone())
            .unwrap_or_default();

        let usage = response.usage.map(|u| Usage {
            prompt_tokens: u.prompt_tokens,
            completion_tokens: u.completion_tokens,
            total_tokens: u.total_tokens,
        });

        Ok(CompletionResponse { content, usage })
    }

    // -----------------------------------------------------------------------
    // Health check
    // -----------------------------------------------------------------------

    pub(super) async fn ping_inner(&self) -> Result<(), ProviderError> {
        // Simple HEAD-style check: try a minimal chat completion
        let request = CreateChatCompletionRequestArgs::default()
            .model("dummy")
            .messages(vec![
                ChatCompletionRequestMessage::User(
                    ChatCompletionRequestUserMessageArgs::default()
                        .content("ping")
                        .build()
                        .expect("invalid"),
                ),
            ])
            .max_completion_tokens(1u32)
            .build()
            .expect("invalid");

        match self.client.chat().create(request).await {
            Ok(_) => Ok(()),
            Err(e) => Err(ProviderError::Network(std::io::Error::new(
                std::io::ErrorKind::ConnectionRefused,
                e,
            ))),
        }
    }
}

// ---------------------------------------------------------------------------
// ProviderTrait implementation
// ---------------------------------------------------------------------------

#[async_trait::async_trait]
impl ProviderTrait for LlamaCppProvider {
    fn clone_box(&self) -> Box<dyn ProviderTrait> {
        Box::new(self.clone_provider())
    }

    async fn stream_chat(
        &self,
        req: &ChatRequest,
        tx: mpsc::Sender<ChatChunk>,
        cancel_rx: watch::Receiver<bool>,
    ) -> Result<(), ProviderError> {
        self.stream_chat_inner(req, tx, cancel_rx).await
    }

    async fn complete(&self, req: &ChatRequest) -> Result<CompletionResponse, ProviderError> {
        self.complete_inner(req).await
    }

    async fn ping(&self) -> Result<(), ProviderError> {
        self.ping_inner().await
    }
}

// ---------------------------------------------------------------------------
// Request building
// ---------------------------------------------------------------------------

/// Convert a normalized `ChatRequest` to an `OpenAI` `CreateChatCompletionRequest`.
fn build_openai_request(
    req: &ChatRequest,
    stream: bool,
) -> CreateChatCompletionRequest {
    let chat_messages: Vec<ChatCompletionRequestMessage> = req
        .messages
        .iter()
        .filter_map(to_openai_message)
        .collect();

    let tools: Vec<ChatCompletionTools> = req
        .tools
        .iter()
        .map(|def| ChatCompletionTools::Function(ChatCompletionTool {
            function: FunctionObject {
                name: def.name.clone(),
                description: Some(def.description.clone()),
                parameters: Some(def.parameters.clone()),
                strict: None, // omitted from JSON via skip_serializing_if (matches pi-coding-agent)
            },
        }))
        .collect();

    let mut req_builder = CreateChatCompletionRequestArgs::default();
    req_builder.model(&req.model);
    req_builder.messages(chat_messages);
    req_builder.tools(tools);
    req_builder.stream(stream);

    if let Some(max_tokens) = req.max_tokens {
        req_builder.max_completion_tokens(max_tokens);
    }

    let mut request = req_builder.build().expect("invalid request");

    // Always include usage in the final stream chunk.
    // This ensures llama.cpp sends a proper finish_reason and token usage
    // in a dedicated trailing chunk, matching what fantasy/crush does.
    // See: OpenAI stream_options: { include_usage: true }
    if stream {
        request.stream_options = Some(
            async_openai::types::chat::ChatCompletionStreamOptions {
                include_usage: Some(true),
                include_obfuscation: None,
            }
        );
    }

    request
}

/// Inject `reasoning_content` into assistant messages in the serialized request JSON.
///
/// `async-openai` 0.41.0 doesn't support `reasoning_content` on
/// `ChatCompletionRequestAssistantMessage`, so we build the request normally
/// and then post-process the JSON to add the field where needed.
/// Modeled on pi-coding-agent's approach (openai-completions.js).
fn inject_reasoning_content(
    request_json: serde_json::Value,
    original_messages: &[Message],
) -> serde_json::Value {
    let mut request = request_json;
    let msg_pairs: Vec<(usize, &Message)> = original_messages.iter().enumerate().collect();

    if let Some(messages) = request.get_mut("messages").and_then(|m| m.as_array_mut()) {
        let mut msg_idx = 0;
        for (_orig_idx, orig_msg) in msg_pairs {
            // to_openai_message skips empty assistant messages, so we need to
            // align original messages with the filtered list.
            if matches!(orig_msg, Message::Assistant { content, tool_calls, .. }
                if content.is_empty() && tool_calls.is_empty())
            {
                continue;
            }
            if msg_idx < messages.len()
                && let Message::Assistant { reasoning, .. } = orig_msg
                && !reasoning.is_empty()
            {
                messages[msg_idx]["reasoning_content"] =
                    serde_json::Value::String(reasoning.clone());
            }
            msg_idx += 1;
        }
    }

    request
}

/// Convert a normalized `Message` to an `OpenAI` `ChatCompletionRequestMessage`.
///
/// Returns `None` for assistant messages with no content and no tool calls
/// (invalid for the `OpenAI` API — some providers reject or silently drop them).
///
/// Note: `reasoning` field is NOT included here because `async-openai` 0.41.0
/// doesn't support `reasoning_content` on `ChatCompletionRequestAssistantMessage`.
/// The reasoning is injected post-hoc in `inject_reasoning_content()`.
pub(crate) fn to_openai_message(msg: &Message) -> Option<ChatCompletionRequestMessage> {
    match msg {
        Message::System { content } => Some(ChatCompletionRequestMessage::System(
            ChatCompletionRequestSystemMessageArgs::default()
                .content(content.clone())
                .build()
                .expect("invalid system message"),
        )),
        Message::User { content } => Some(ChatCompletionRequestMessage::User(
            ChatCompletionRequestUserMessageArgs::default()
                .content(content.clone())
                .build()
                .expect("invalid user message"),
        )),
        Message::Assistant {
            content,
            tool_calls,
            ..
        } => {
            // Guard: skip assistant messages with no content, no reasoning, and no tool calls.
            // OpenAI API requires "either content or tool_calls, but not none".
            // Some providers silently drop or error on empty assistant messages.
            // Modeled on pi-coding-agent's convertMessages() guard.
            let has_reasoning = false; // reasoning handled separately via JSON injection
            if content.is_empty() && tool_calls.is_empty() && !has_reasoning {
                return None;
            }
            let mut builder = ChatCompletionRequestAssistantMessageArgs::default();
            // Match pi-coding-agent: omit `content` when empty.
            // Pi-coding-agent sends `content: null` (omitted via skip_serializing_if)
            // for assistant messages with tool_calls but no text. Respondami previously
            // sent `content: ""` which some LLM servers (llama.cpp + qwen3) interpret
            // as "assistant said nothing", increasing probability of empty responses.
            // See: pi-ai/dist/api/openai-completions.js line 736:
            //   content: compat.requiresAssistantAfterToolResult ? "" : null
            if !content.is_empty() {
                builder.content(content.clone());
            }
            if !tool_calls.is_empty() {
                let openai_calls: Vec<ChatCompletionMessageToolCalls> = tool_calls
                    .iter()
                    .map(|tc| {
                        ChatCompletionMessageToolCalls::Function(ChatCompletionMessageToolCall {
                            id: tc.id.clone(),
                            function: FunctionCall {
                                name: tc.name.clone(),
                                arguments: tc.arguments.to_string(),
                            },
                        })
                    })
                    .collect();
                builder.tool_calls(openai_calls);
            }
            Some(ChatCompletionRequestMessage::Assistant(
                builder.build().expect("invalid assistant message"),
            ))
        }
        Message::Tool {
            tool_call_id,
            result,
        } => Some(ChatCompletionRequestMessage::Tool(
            ChatCompletionRequestToolMessageArgs::default()
                .tool_call_id(tool_call_id.clone())
                .content(result.clone())
                .build()
                .expect("invalid tool message"),
        )),
    }
}
