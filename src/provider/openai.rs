//! OpenAI-compatible provider implementation.
//!
//! Maps `OpenAI` Chat Completions API to the normalized provider abstraction.
//! Serves llama.cpp, Ollama, vLLM, and `OpenAI` endpoints. The `LlamaCpp`
//! provider (see `llamacpp.rs`) wraps this one to add llama.cpp-specific
//! model autodetection.

use async_openai::Client;
use async_openai::config::OpenAIConfig;
use async_openai::types::chat::{
    ChatCompletionMessageToolCall, ChatCompletionMessageToolCalls,
    ChatCompletionRequestAssistantMessageArgs, ChatCompletionRequestMessage,
    ChatCompletionRequestMessageContentPartImage, ChatCompletionRequestMessageContentPartText,
    ChatCompletionRequestSystemMessageArgs, ChatCompletionRequestToolMessageArgs,
    ChatCompletionRequestUserMessageArgs, ChatCompletionRequestUserMessageContent,
    ChatCompletionRequestUserMessageContentPart, ChatCompletionTool, ChatCompletionTools,
    CreateChatCompletionRequest, CreateChatCompletionRequestArgs, CreateChatCompletionResponse,
    FunctionCall, FunctionObject, ImageUrl,
};

use super::{
    ChatChunk, ChatRequest, CompletionResponse, DiscoveredModel, OpenAISettings, Message,
    ProviderError, ProviderTrait, is_context_overflow,
};
use crate::session::Usage;
use std::sync::Mutex;
use std::time::Instant;
use tokio::sync::mpsc;
use tokio::sync::watch;

/// User-Agent string for HTTP requests to the provider.
const USER_AGENT: &str = concat!("Respondami/", env!("CARGO_PKG_VERSION"));

/// 30s cache for `/v1/models` response bodies, keyed by endpoint URL. Avoids
/// hammering the endpoint on every `/model` picker open. Stores the raw body
/// so the `OpenAI` and `LlamaCpp` providers can each parse it their way.
static MODEL_CACHE: Mutex<Option<(String, Instant, String)>> = Mutex::new(None);
const MODEL_CACHE_TTL: std::time::Duration = std::time::Duration::from_secs(30);

/// Provider for OpenAI-compatible endpoints (llama.cpp, Ollama, vLLM, `OpenAI`).
///
/// `url`/`api_key` duplicate data held inside `client`'s `OpenAIConfig`, but
/// async-openai 0.42.2 exposes no public getters for them, and the raw
/// reqwest streaming path needs both — the duplication is unavoidable until
/// the dependency grows accessors.
pub struct OpenAIProvider {
    url: String,
    api_key: String,
    client: Client<OpenAIConfig>,
    http_client: reqwest::Client,
}

impl std::fmt::Debug for OpenAIProvider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OpenAIProvider")
            .field("url", &self.url)
            .field("api_key", &"[redacted]")
            .finish()
    }
}

impl OpenAIProvider {
    /// Create an OpenAI-compatible provider from settings.
    ///
    /// # Panics
    ///
    /// - If `reqwest::Client::builder().build()` fails (extremely rare, only on
    ///   misconfigured HTTP stack).
    #[must_use]
    pub fn from_settings(settings: &OpenAISettings) -> Self {
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

        let mut req_builder = self
            .http_client
            .post(&url)
            .header("Content-Type", "application/json")
            .header("Accept", "text/event-stream")
            .body(request_body);

        if !self.api_key.is_empty() {
            req_builder = req_builder.header("Authorization", format!("Bearer {}", self.api_key));
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
            return Err(ProviderError::Http {
                status,
                body,
                retry_after,
            });
        }

        super::sse::parse_sse_stream(
            response,
            tx,
            cancel_rx,
            crate::sse_debug::current_turn().as_ref(),
        )
        .await
    }

    // -----------------------------------------------------------------------
    // Non-streaming completion (for compaction)
    // -----------------------------------------------------------------------

    pub(super) async fn complete_inner(
        &self,
        req: &ChatRequest,
    ) -> Result<CompletionResponse, ProviderError> {
        let request = build_openai_request(req, false);

        let response = self
            .client
            .chat()
            .create(request)
            .await
            .map_err(|e| ProviderError::Network(std::io::Error::other(e)))?;

        parse_completion_response(response)
    }

    // -----------------------------------------------------------------------
    // Model discovery
    // -----------------------------------------------------------------------

    /// Fetch the raw `/v1/models` response body, using the 30s cache.
    ///
    /// Exposed to the `LlamaCpp` provider so it can rich-parse the same body
    /// without re-fetching. The `OpenAI` provider parses it with the basic
    /// parser (no `loaded`/`vision`).
    pub(crate) async fn fetch_models_body(&self) -> Result<String, ProviderError> {
        // Check the 30s cache first (keyed by endpoint URL).
        {
            let guard = MODEL_CACHE.lock().unwrap();
            if let Some((url, at, body)) = guard.as_ref()
                && url == &self.url
                && at.elapsed() < MODEL_CACHE_TTL
            {
                return Ok(body.clone());
            }
        }

        let endpoint = models_endpoint_url(&self.url);
        let mut req_builder = self.http_client.get(&endpoint);
        if !self.api_key.is_empty() {
            req_builder = req_builder.header("Authorization", format!("Bearer {}", self.api_key));
        }
        let response = req_builder
            .timeout(std::time::Duration::from_secs(5))
            .send()
            .await
            .map_err(|e| ProviderError::Network(std::io::Error::other(e)))?;

        if !response.status().is_success() {
            let status = response.status().as_u16();
            let body = response.text().await.unwrap_or_default();
            return Err(ProviderError::Http {
                status,
                body,
                retry_after: None,
            });
        }

        let body = response.text().await.unwrap_or_default();

        // Store the raw body in the cache.
        {
            let mut guard = MODEL_CACHE.lock().unwrap();
            *guard = Some((self.url.clone(), Instant::now(), body.clone()));
        };

        Ok(body)
    }

    /// List models, parsing the body with the basic parser (no `loaded`/`vision`).
    pub(super) async fn list_models_inner(&self) -> Result<Vec<DiscoveredModel>, ProviderError> {
        let body = self.fetch_models_body().await?;
        Ok(parse_models_basic(&body))
    }

    // -----------------------------------------------------------------------
    // Health check
    // -----------------------------------------------------------------------

    pub(super) async fn ping_inner(&self, model: &str) -> Result<(), ProviderError> {
        // Simple HEAD-style check: try a minimal chat completion with the
        // configured model (some endpoints reject unknown model names).
        let request = CreateChatCompletionRequestArgs::default()
            .model(model)
            .messages(vec![ChatCompletionRequestMessage::User(
                ChatCompletionRequestUserMessageArgs::default()
                    .content("ping")
                    .build()
                    .expect("invalid"),
            )])
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

/// Map a non-streaming completion response to the normalized form.
///
/// Empty `choices` is an error, not a silent empty string: the LLM compaction
/// path handles `Err` with a no-summary fallback, so a malformed response must
/// not masquerade as an empty summary.
///
/// # Errors
///
/// - [`ProviderError::EmptyResponse`] when the response carries no choices.
pub(crate) fn parse_completion_response(
    response: CreateChatCompletionResponse,
) -> Result<CompletionResponse, ProviderError> {
    let content = response
        .choices
        .first()
        .and_then(|c| c.message.content.clone())
        .ok_or_else(|| ProviderError::EmptyResponse {
            detail: "non-streaming completion returned no choices".to_string(),
        })?;

    let usage = response.usage.map(|u| Usage {
        prompt_tokens: u.prompt_tokens,
        completion_tokens: u.completion_tokens,
        total_tokens: u.total_tokens,
    });

    Ok(CompletionResponse { content, usage })
}

/// Build the `/v1/models` endpoint URL from the base URL.
///
/// Handles base URLs with or without a trailing `/v1` and with or without a
/// trailing slash. Examples:
/// - `http://localhost:8080` → `http://localhost:8080/v1/models`
/// - `http://localhost:8080/v1` → `http://localhost:8080/v1/models`
/// - `http://localhost:8080/v1/` → `http://localhost:8080/v1/models`
#[must_use]
pub(crate) fn models_endpoint_url(base_url: &str) -> String {
    let trimmed = base_url.trim_end_matches('/');
    if trimmed.ends_with("/v1") {
        format!("{trimmed}/models")
    } else {
        format!("{trimmed}/v1/models")
    }
}

/// Parse a `/v1/models` response body into a list of [`DiscoveredModel`]s
/// using the **basic** parser: name + context length only. `loaded` and
/// `vision` are left `None` (the generic OpenAI-compatible endpoint does not
/// report them).
///
/// Tolerant: returns an empty list on any parse failure. Each entry is read
/// from `data[].id`; the context length is read from `data[].context_length`
/// (falling back to `data[].max_context_length`).
#[must_use]
pub(crate) fn parse_models_basic(body: &str) -> Vec<DiscoveredModel> {
    let Ok(value) = serde_json::from_str::<serde_json::Value>(body) else {
        return Vec::new();
    };
    let Some(data) = value.get("data").and_then(|d| d.as_array()) else {
        return Vec::new();
    };
    data.iter()
        .filter_map(|entry| {
            let name = entry.get("id")?.as_str()?.to_string();
            if name.is_empty() {
                return None;
            }
            let context_length = entry
                .get("context_length")
                .or_else(|| entry.get("max_context_length"))
                .and_then(serde_json::Value::as_u64)
                .map(|c| u32::try_from(c).unwrap_or(u32::MAX));
            Some(DiscoveredModel {
                name,
                context_length,
                loaded: None,
                vision: None,
            })
        })
        .collect()
}

/// Parse a `/v1/models` response body into a list of [`DiscoveredModel`]s
/// using the **rich** llama.cpp parser: name + context length + `loaded`
/// (`status.value`) + `vision` (`architecture.input_modalities` contains
/// `"image"`).
///
/// Tolerant: a missing `status`/`architecture`/context field yields `None` for
/// that attribute (the caller applies graceful defaults). Malformed entries
/// (no `id`) are skipped. Returns an empty list on a non-array `data` or a
/// non-JSON body.
#[must_use]
pub(crate) fn parse_models_rich(body: &str) -> Vec<DiscoveredModel> {
    let Ok(value) = serde_json::from_str::<serde_json::Value>(body) else {
        return Vec::new();
    };
    let Some(data) = value.get("data").and_then(|d| d.as_array()) else {
        return Vec::new();
    };
    data.iter()
        .filter_map(|entry| {
            let name = entry.get("id")?.as_str()?.to_string();
            if name.is_empty() {
                return None;
            }
            let loaded = entry
                .get("status")
                .and_then(|s| s.get("value"))
                .and_then(serde_json::Value::as_str)
                .map(|v| v == "loaded");
            let vision = entry
                .get("architecture")
                .and_then(|a| a.get("input_modalities"))
                .and_then(serde_json::Value::as_array)
                .map(|mods| mods.iter().any(|m| m.as_str() == Some("image")));
            let context_length = entry
                .get("context_length")
                .or_else(|| entry.get("max_context_length"))
                .and_then(serde_json::Value::as_u64)
                .or_else(|| {
                    entry
                        .get("meta")
                        .and_then(|m| m.get("n_ctx"))
                        .and_then(serde_json::Value::as_u64)
                })
                .or_else(|| ctx_size_from_args(entry))
                .map(|c| u32::try_from(c).unwrap_or(u32::MAX));
            Some(DiscoveredModel {
                name,
                context_length,
                loaded,
                vision,
            })
        })
        .collect()
}

/// Extract the `--ctx-size` value from a model's `status.args` array (the
/// llama.cpp router embeds the full launch args). The value may be a JSON
/// number or a numeric string; returns `None` when the arg is absent or not a
/// number.
fn ctx_size_from_args(entry: &serde_json::Value) -> Option<u64> {
    let args = entry.get("status")?.get("args")?.as_array()?;
    let mut i = 0;
    while i < args.len() {
        if args[i].as_str() == Some("--ctx-size")
            && let Some(next) = args.get(i + 1)
        {
            // The value may be a JSON number or a numeric string.
            if let Some(n) = next.as_u64().or_else(|| next.as_str().and_then(|s| s.parse().ok())) {
                return Some(n);
            }
        }
        i += 1;
    }
    None
}

// ---------------------------------------------------------------------------
// ProviderTrait implementation
// ---------------------------------------------------------------------------

#[async_trait::async_trait]
impl ProviderTrait for OpenAIProvider {
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

    async fn ping(&self, model: &str) -> Result<(), ProviderError> {
        self.ping_inner(model).await
    }

    async fn list_models(&self) -> Result<Vec<DiscoveredModel>, ProviderError> {
        self.list_models_inner().await
    }
}

// ---------------------------------------------------------------------------
// Request building
// ---------------------------------------------------------------------------

/// Convert a normalized `ChatRequest` to an `OpenAI` `CreateChatCompletionRequest`.
fn build_openai_request(req: &ChatRequest, stream: bool) -> CreateChatCompletionRequest {
    let chat_messages: Vec<ChatCompletionRequestMessage> =
        req.messages.iter().filter_map(to_openai_message).collect();

    let tools: Vec<ChatCompletionTools> = req
        .tools
        .iter()
        .map(|def| {
            ChatCompletionTools::Function(ChatCompletionTool {
                function: FunctionObject {
                    name: def.name.clone(),
                    description: Some(def.description.clone()),
                    parameters: Some(def.parameters.clone()),
                    strict: None, // omitted from JSON via skip_serializing_if (matches pi-coding-agent)
                },
            })
        })
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
        request.stream_options = Some(async_openai::types::chat::ChatCompletionStreamOptions {
            include_usage: Some(true),
            include_obfuscation: None,
        });
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
            // to_openai_message skips assistant messages with no content, no
            // reasoning, and no tool calls, so we need to align original
            // messages with the filtered list using the same three-way guard.
            if matches!(orig_msg, Message::Assistant { content, tool_calls, reasoning, .. }
                if content.is_empty() && tool_calls.is_empty() && reasoning.is_empty())
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
        Message::User { content, images } => {
            // Vision: when the message carries images, use the array content form
            // (data-URL image parts). Text-only messages keep the plain string form.
            let content_value = if images.is_empty() {
                ChatCompletionRequestUserMessageContent::Text(content.clone())
            } else {
                // A text part must accompany the image parts: an image-only
                // content array leaves the user turn without any text, which
                // local models (llama.cpp + qwen3) interpret as a user turn
                // that said nothing, increasing the probability of empty
                // responses. The tool result's own text note already describes
                // the image; this part just makes the turn non-empty.
                let mut parts: Vec<ChatCompletionRequestUserMessageContentPart> =
                    vec![ChatCompletionRequestUserMessageContentPart::Text(
                        ChatCompletionRequestMessageContentPartText {
                            text: "Image attached.".to_string(),
                            prompt_cache_breakpoint: None,
                        },
                    )];
                parts.extend(images.iter().map(|img| {
                    ChatCompletionRequestUserMessageContentPart::ImageUrl(
                        ChatCompletionRequestMessageContentPartImage {
                            image_url: ImageUrl {
                                url: format!("data:{};base64,{}", img.mime_type, img.data),
                                detail: None,
                            },
                            prompt_cache_breakpoint: None,
                        },
                    )
                }));
                ChatCompletionRequestUserMessageContent::Array(parts)
            };
            Some(ChatCompletionRequestMessage::User(
                ChatCompletionRequestUserMessageArgs::default()
                    .content(content_value)
                    .build()
                    .expect("invalid user message"),
            ))
        }
        Message::Assistant {
            content,
            tool_calls,
            reasoning,
        } => {
            // Guard: skip assistant messages with no content, no reasoning, and
            // no tool calls. OpenAI API requires "either content or tool_calls,
            // but not none". Reasoning-only messages are kept: the reasoning is
            // injected post-hoc as `reasoning_content` in `inject_reasoning_content`.
            // Some providers silently drop or error on empty assistant messages.
            // Modeled on pi-coding-agent's convertMessages() guard.
            if content.is_empty() && tool_calls.is_empty() && reasoning.is_empty() {
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
