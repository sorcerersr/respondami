//! Configuration and model context state.
//!
//! Holds `Config`, `model_name`, `context_window`, display modes (thinking, tool
//! output, hooks), and runtime context: `cwd`, `rtk_state`, `skills`,
//! `agents_md_path`, and `hook_registry`.

use std::path::PathBuf;

use super::hook_display::HookDisplay;
use super::thinking_display::ThinkingDisplay;
use crate::config::{Config, ModelConfig};
use crate::hooks::HookRegistry;
use crate::provider::DiscoveredModel;
use crate::skills::Skill;
use crate::tools::rtk::RtkState;

/// Configuration and model context state.
///
/// `model_name`/`context_window`/`vision`/`loaded`/`discovered_models` are the
/// single source of truth for the *live* active model. For the `openai`
/// provider they mirror `config.provider`; for `llamacpp` they are resolved
/// from `discovered_models` (autodetected at runtime). `live_config()` injects
/// them into a `Config` snapshot so the agent loop and compaction read the live
/// values without any change to their code paths.
#[derive(Debug)]
pub struct ConfigState {
    pub config: Config,
    pub model_name: String,
    pub context_window: u32,
    /// Whether the active model supports vision. `openai`: from its per-model
    /// config flag. `llamacpp`: from `discovered_models` (default `false`).
    pub vision: bool,
    /// Whether the active model is currently loaded on the server. `None` for
    /// the `openai` provider (concept doesn't apply) and before the first
    /// `llamacpp` fetch.
    pub loaded: Option<bool>,
    /// Last `/v1/models` result (llamacpp only; empty for openai). Used by the
    /// status bar and picker to render loaded/vision icons without re-fetching.
    pub discovered_models: Vec<DiscoveredModel>,
    pub thinking_display: ThinkingDisplay,
    pub tool_output_expanded: bool,
    pub hook_display: HookDisplay,
    pub cwd: PathBuf,
    pub rtk_state: RtkState,
    pub skills: Vec<Skill>,
    pub agents_md_path: Option<PathBuf>,
    pub hook_registry: HookRegistry,
}

impl ConfigState {
    /// Return a `Config` snapshot with the live model injected into
    /// `provider`, so the agent loop and compaction read the live model's name,
    /// context window, and vision flag. The stored `config` is not mutated.
    #[must_use]
    pub fn live_config(&self) -> Config {
        let mut config = self.config.clone();
        config.provider.active_model = self.model_name.clone();
        if let Some(m) = config
            .provider
            .models
            .iter_mut()
            .find(|m| m.name == self.model_name)
        {
            m.context_window = self.context_window;
            m.vision = self.vision;
        } else {
            // `llamacpp` has no configured models; inject a synthetic entry so
            // `active_vision()`/`active_context_window()` resolve to the live
            // values.
            config.provider.models.push(ModelConfig {
                name: self.model_name.clone(),
                context_window: self.context_window,
                vision: self.vision,
            });
        }
        config
    }

    /// Resolve the active model's `(context_window, vision, loaded)` from
    /// `discovered_models` using the design's graceful fallbacks: `vision`
    /// defaults to `false` (never send images a model may not see),
    /// `context_window` to `32768` (corrected later by the first real
    /// `prompt_tokens`), `loaded` stays `None` when not reported.
    ///
    /// Returns `None` when the model is not in the list (caller keeps the
    /// current state).
    #[must_use]
    pub fn resolve_discovered(&self, name: &str) -> Option<(u32, bool, Option<bool>)> {
        let dm = self.discovered_models.iter().find(|m| m.name == name)?;
        let vision = dm.vision.unwrap_or(false);
        let context_window = dm.context_length.unwrap_or(32768);
        Some((context_window, vision, dm.loaded))
    }
}
