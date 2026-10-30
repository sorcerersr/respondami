//! Llama.cpp provider — wraps the OpenAI-compatible provider to add
//! llama.cpp-specific model autodetection.
//!
//! The llama.cpp server / router reports, per model, whether it is currently
//! `loaded` (`status.value`) and whether it supports vision
//! (`architecture.input_modalities` contains `"image"`). This provider parses
//! those from the same `/v1/models` body the `OpenAI` provider uses, so the
//! TUI can show loaded/vision state and the agent can resolve the context
//! window and vision gate without any manual model config.
//!
//! Streaming, completion, and ping are delegated to the underlying
//! [`OpenAIProvider`].

use super::openai::OpenAIProvider;
use super::{
    ChatChunk, ChatRequest, CompletionResponse, DiscoveredModel, LlamaCppSettings, OpenAISettings,
    ProviderError, ProviderTrait,
};
use tokio::sync::mpsc;
use tokio::sync::watch;

/// Provider for llama.cpp servers / routers.
///
/// Wraps an [`OpenAIProvider`] (shared streaming/request machinery) and adds
/// rich `/v1/models` parsing for `loaded`/`vision`/context length.
pub struct LlamaCppProvider {
    inner: OpenAIProvider,
}

impl std::fmt::Debug for LlamaCppProvider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LlamaCppProvider")
            .field("inner", &self.inner)
            .finish()
    }
}

impl LlamaCppProvider {
    /// Create a llama.cpp provider from settings.
    #[must_use]
    pub fn from_settings(settings: &LlamaCppSettings) -> Self {
        let openai_settings = OpenAISettings {
            url: settings.url.clone(),
            api_key: settings.api_key.clone(),
        };
        Self {
            inner: OpenAIProvider::from_settings(&openai_settings),
        }
    }

    /// Clone the provider (rebuilds the inner `OpenAI` provider).
    #[must_use]
    pub fn clone_provider(&self) -> Self {
        Self {
            inner: self.inner.clone_provider(),
        }
    }

    /// List models, rich-parsing the `/v1/models` body for `loaded`/`vision`
    /// and the context length.
    pub(super) async fn list_models_inner(&self) -> Result<Vec<DiscoveredModel>, ProviderError> {
        let body = self.inner.fetch_models_body().await?;
        Ok(super::openai::parse_models_rich(&body))
    }
}

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
        self.inner.stream_chat(req, tx, cancel_rx).await
    }

    async fn complete(&self, req: &ChatRequest) -> Result<CompletionResponse, ProviderError> {
        self.inner.complete(req).await
    }

    async fn ping(&self, model: &str) -> Result<(), ProviderError> {
        self.inner.ping(model).await
    }

    async fn list_models(&self) -> Result<Vec<DiscoveredModel>, ProviderError> {
        self.list_models_inner().await
    }
}
