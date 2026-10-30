//! Application configuration — loading, validation, and schema migration.
//!
//! Reads `config.json` from `~/.config/respondami/`. Handles legacy config
//! migration, provider settings, compaction thresholds, UI display modes, and
//! RTK integration settings.

use anyhow::Context;
use serde::ser::Error;
use serde::{Deserialize, Deserializer, Serialize};
use std::fs;
use std::path::PathBuf;

use crate::provider::{LlamaCppSettings, OpenAISettings, ProviderSettings};
use crate::tui::{HookDisplay, ThinkingDisplay};

/// Per-model configuration: the model name plus its context window and
/// vision support. A provider can expose many models; each carries its own
/// `context_window` and `vision` so switching models updates both.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ModelConfig {
    /// The exact model name sent in requests.
    pub name: String,
    /// The model's context window in tokens.
    pub context_window: u32,
    /// Whether this model supports vision (images). When false, the `read`
    /// tool returns a text note instead of an image attachment.
    pub vision: bool,
}

/// Provider configuration for connecting to an LLM.
///
/// Uses an internally-tagged `settings` enum (`type` discriminator in JSON).
/// Legacy configs (without `type`) are auto-migrated to `llamacpp` on load.
#[derive(Debug, Clone)]
pub struct ProviderConfig {
    /// Provider-specific settings (url, `api_key`, etc.).
    pub settings: ProviderSettings,
    /// Last-used model; the default for new sessions. Updated by the picker.
    pub active_model: String,
    /// Known models and their per-model config, keyed by `name`.
    pub models: Vec<ModelConfig>,
    /// Workaround for server/provider environments where images in the
    /// conversation interfere with prompt caching. When true, an image is
    /// removed from the context after the model has seen it (a text note is
    /// left in its place; the file can be re-read).
    pub strip_images_after_view: bool,
}

impl ProviderConfig {
    /// Get the provider type name.
    #[must_use]
    pub fn provider_type(&self) -> &str {
        match &self.settings {
            ProviderSettings::OpenAI(_) => "openai",
            ProviderSettings::LlamaCpp(_) => "llamacpp",
        }
    }

    /// Look up a model's config by name. `None` if not in the list.
    #[must_use]
    pub fn model_config(&self, name: &str) -> Option<&ModelConfig> {
        self.models.iter().find(|m| m.name == name)
    }

    /// The active model's config, falling back to the first model.
    #[must_use]
    pub fn active_model_config(&self) -> Option<&ModelConfig> {
        self.model_config(&self.active_model)
            .or_else(|| self.models.first())
    }

    /// The active model's context window (falls back to the first model, then 32768).
    #[must_use]
    pub fn active_context_window(&self) -> u32 {
        self.active_model_config()
            .map(|m| m.context_window)
            .unwrap_or(32768)
    }

    /// The active model's vision flag (falls back to the first model, then true).
    #[must_use]
    pub fn active_vision(&self) -> bool {
        self.active_model_config().map(|m| m.vision).unwrap_or(true)
    }

    /// Resolve a model's `(context_window, vision)` by name, falling back to
    /// the given defaults when the model is not configured (e.g. a newly
    /// discovered model that has not been added to `models` yet).
    #[must_use]
    pub fn model_config_or(&self, name: &str, default_ctx: u32) -> (u32, bool) {
        match self.model_config(name) {
            Some(m) => (m.context_window, m.vision),
            None => (default_ctx, true),
        }
    }
}

impl<'de> Deserialize<'de> for ProviderConfig {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        use serde::de::Error;

        let value = serde_json::Value::deserialize(deserializer)?;
        let obj = value
            .as_object()
            .ok_or_else(|| D::Error::custom("expected provider config object"))?;

        let url = obj
            .get("url")
            .and_then(|v| v.as_str())
            .unwrap_or("http://localhost:8080")
            .to_string();
        let api_key = obj
            .get("api_key")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let strip_images_after_view = obj
            .get("strip_images_after_view")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false);

        // Determine the provider type and whether a legacy model block is
        // present. A `type: llamacpp` config that still carries a model block
        // (`models`/`active_model`/`model`/`context_window`/`vision`) is legacy
        // and is migrated to `openai`. A `type: llamacpp` config with no model
        // block is the new autodetecting provider. A no-type config is the
        // oldest legacy shape and also migrates to `openai`.
        let has_model_block = ["models", "active_model", "model", "context_window", "vision"]
            .iter()
            .any(|k| obj.get(*k).is_some());

        let provider_is_llamacpp = match obj.get("type").and_then(|v| v.as_str()) {
            Some("llamacpp") => !has_model_block,
            _ => false,
        };

        // Build the models list. For the new `llamacpp` provider there is no
        // model block — models are autodetected at runtime, so the list stays
        // empty. For `openai`, prefer the `models` array; fall back to a single
        // legacy `model`/`context_window`/`vision` entry for backward
        // compatibility with pre-multi-model configs.
        let models: Vec<ModelConfig> = if provider_is_llamacpp {
            Vec::new()
        } else if let Some(arr) = obj.get("models").and_then(|v| v.as_array()) {
            arr.iter()
                .filter_map(|m| {
                    let name = m.get("name")?.as_str()?.to_string();
                    if name.is_empty() {
                        return None;
                    }
                    let context_window = m
                        .get("context_window")
                        .and_then(serde_json::Value::as_u64)
                        .map_or(32768, |v| u32::try_from(v).unwrap_or(u32::MAX));
                    let vision = m
                        .get("vision")
                        .and_then(serde_json::Value::as_bool)
                        .unwrap_or(true);
                    Some(ModelConfig {
                        name,
                        context_window,
                        vision,
                    })
                })
                .collect()
        } else {
            // Legacy: single top-level model entry.
            let legacy_model = obj
                .get("model")
                .and_then(|v| v.as_str())
                .unwrap_or("llama3.2");
            let legacy_ctx = obj
                .get("context_window")
                .and_then(serde_json::Value::as_u64)
                .map_or(32768, |v| u32::try_from(v).unwrap_or(u32::MAX));
            let legacy_vision = obj
                .get("vision")
                .and_then(serde_json::Value::as_bool)
                .unwrap_or(true);
            vec![ModelConfig {
                name: legacy_model.to_string(),
                context_window: legacy_ctx,
                vision: legacy_vision,
            }]
        };

        // The active model: prefer the new `active_model` key, fall back to the
        // legacy `model` key, then to the first configured model. For the new
        // `llamacpp` provider this stays empty (resolved at startup from the
        // server's loaded model).
        let active_model = if provider_is_llamacpp {
            String::new()
        } else {
            obj.get("active_model")
                .and_then(|v| v.as_str())
                .or_else(|| obj.get("model").and_then(|v| v.as_str()))
                .map(str::to_string)
                .unwrap_or_else(|| models.first().map(|m| m.name.clone()).unwrap_or_default())
        };

        // Build settings based on `type` field. `llamacpp` with a model block
        // (or no type) migrates to `openai`; a bare `llamacpp` is the new
        // autodetecting provider.
        let settings = match obj.get("type").and_then(|v| v.as_str()) {
            Some("anthropic") => {
                // Future: parse AnthropicSettings
                return Err(D::Error::custom("anthropic provider not yet implemented"));
            }
            Some("llamacpp") if provider_is_llamacpp => {
                ProviderSettings::LlamaCpp(LlamaCppSettings { url, api_key })
            }
            Some("openai" | "llamacpp") | None => {
                ProviderSettings::OpenAI(OpenAISettings { url, api_key })
            }
            Some(other) => {
                return Err(D::Error::custom(format!("unknown provider type: {other}")));
            }
        };

        Ok(ProviderConfig {
            settings,
            active_model,
            models,
            strip_images_after_view,
        })
    }
}

impl Serialize for ProviderConfig {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        // Serialize as a flat object with `type` tag from the settings enum
        let mut map = serde_json::Map::new();
        map.insert(
            "type".to_string(),
            serde_json::Value::String(self.provider_type().to_string()),
        );

        // Add provider-specific fields (both variants carry url + api_key).
        let s_val = match &self.settings {
            ProviderSettings::OpenAI(s) => serde_json::to_value(s),
            ProviderSettings::LlamaCpp(s) => serde_json::to_value(s),
        }
        .map_err(S::Error::custom)?;
        if let Some(obj) = s_val.as_object() {
            for (k, v) in obj {
                map.insert(k.clone(), v.clone());
            }
        }

        // The `llamacpp` provider autodetects its models at runtime, so it has
        // no `active_model`/`models` in the config file. The `openai` provider
        // persists them.
        let is_llamacpp = matches!(self.settings, ProviderSettings::LlamaCpp(_));
        if !is_llamacpp {
            map.insert(
                "active_model".to_string(),
                serde_json::Value::String(self.active_model.clone()),
            );
            let models_val = serde_json::to_value(&self.models).map_err(S::Error::custom)?;
            map.insert("models".to_string(), models_val);
        }
        map.insert(
            "strip_images_after_view".to_string(),
            serde_json::Value::Bool(self.strip_images_after_view),
        );

        map.serialize(serializer)
    }
}

/// Compaction mode — selects how a compaction plan is produced.
///
/// `Llm` uses the existing model-based summarization. `Vcc` is the new
/// algorithmic (LLM-free) extraction path and the default on upgrade.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CompactionMode {
    /// LLM-based summarization (existing behavior).
    Llm,
    /// Algorithmic extraction, no model call (default).
    Vcc,
}

/// VCC-specific settings. Only read when `CompactionConfig::mode` is `Vcc`.
///
/// Deliberately thin: `smart_keep_tail` and `brief_budget_tokens`. Everything
/// else uses fixed constants ported from the pi-vcc reference.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct VccConfig {
    /// Grow the default `keep:1` tail while it stays small (default true).
    #[serde(default = "default_smart_keep_tail")]
    pub smart_keep_tail: bool,
    /// Token floor for the ranked brief budget (default 1100).
    #[serde(default = "default_brief_budget_tokens")]
    pub brief_budget_tokens: u32,
}

impl Default for VccConfig {
    fn default() -> Self {
        Self {
            smart_keep_tail: default_smart_keep_tail(),
            brief_budget_tokens: default_brief_budget_tokens(),
        }
    }
}

/// Compaction settings.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompactionConfig {
    #[serde(default = "default_compaction_enabled")]
    pub enabled: bool,
    /// Which strategy produces the plan. Defaults to `vcc` on upgrade.
    #[serde(default = "default_compaction_mode")]
    pub mode: CompactionMode,
    #[serde(default = "default_reserve_tokens")]
    pub reserve_tokens: u32,
    #[serde(default = "default_keep_recent_tokens")]
    pub keep_recent_tokens: u32,
    /// VCC-specific settings (only used when `mode` is `Vcc`).
    #[serde(default)]
    pub vcc: VccConfig,
}

/// UI display configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UiConfig {
    #[serde(default)]
    pub thinking_display: ThinkingDisplay,
    #[serde(default = "default_thinking_max_lines")]
    pub thinking_max_lines: usize,
    /// Default expanded state for tool call output.
    /// When false, tool calls show tail view (last N lines).
    /// When true, tool calls show full output.
    #[serde(default = "default_tool_output_expanded")]
    pub tool_output_expanded: bool,
    /// Display mode for hook messages.
    #[serde(default)]
    pub hook_display: HookDisplay,
    /// Show hidden files (dotfiles, dotdirs) in the file autocomplete popup.
    #[serde(default = "default_file_show_hidden")]
    pub file_show_hidden: bool,
    /// Gitignore-style patterns whose matches remain visible in the file
    /// autocomplete popup even when covered by the project `.gitignore`.
    /// The config file value replaces this default wholesale.
    #[serde(default = "default_file_always_visible")]
    pub file_always_visible: Vec<String>,
}

fn default_thinking_max_lines() -> usize {
    5
}

fn default_tool_output_expanded() -> bool {
    true
}

fn default_file_show_hidden() -> bool {
    true
}

fn default_file_always_visible() -> Vec<String> {
    vec!["docs/plans/".to_string(), "docs/reviews/".to_string()]
}

impl Default for UiConfig {
    fn default() -> Self {
        Self {
            thinking_display: ThinkingDisplay::default(),
            thinking_max_lines: default_thinking_max_lines(),
            tool_output_expanded: default_tool_output_expanded(),
            hook_display: HookDisplay::default(),
            file_show_hidden: default_file_show_hidden(),
            file_always_visible: default_file_always_visible(),
        }
    }
}

/// RTK integration configuration.
///
/// Controls command rewrite (via `rtk rewrite`).
/// Output compaction is handled by `rtk` itself.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RtkConfig {
    #[serde(default = "default_rtk_enabled")]
    pub enabled: bool,
}

fn default_rtk_enabled() -> bool {
    true
}

impl Default for RtkConfig {
    fn default() -> Self {
        Self {
            enabled: default_rtk_enabled(),
        }
    }
}

/// Retry configuration for transient provider errors.
///
/// Mirrors pi-coding-agent's retry system: exponential backoff on empty
/// or failed responses, with configurable max attempts and base delay.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RetryConfig {
    #[serde(default = "default_retry_enabled")]
    pub enabled: bool,
    #[serde(default = "default_retry_max_retries")]
    pub max_retries: u32,
    #[serde(default = "default_retry_base_delay_ms")]
    pub base_delay_ms: u64,
}

fn default_retry_enabled() -> bool {
    true
}

fn default_retry_max_retries() -> u32 {
    3
}

fn default_retry_base_delay_ms() -> u64 {
    2000
}

impl Default for RetryConfig {
    fn default() -> Self {
        Self {
            enabled: default_retry_enabled(),
            max_retries: default_retry_max_retries(),
            base_delay_ms: default_retry_base_delay_ms(),
        }
    }
}

/// Full application configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    pub provider: ProviderConfig,
    pub compaction: CompactionConfig,
    #[serde(default)]
    pub ui: UiConfig,
    #[serde(default)]
    pub rtk: RtkConfig,
    #[serde(default)]
    pub retry: RetryConfig,
}

fn default_compaction_enabled() -> bool {
    true
}

fn default_compaction_mode() -> CompactionMode {
    CompactionMode::Vcc
}

fn default_smart_keep_tail() -> bool {
    true
}

fn default_brief_budget_tokens() -> u32 {
    1100
}

fn default_reserve_tokens() -> u32 {
    16384
}

fn default_keep_recent_tokens() -> u32 {
    16384
}

impl Default for Config {
    fn default() -> Self {
        Self {
            provider: ProviderConfig {
                settings: ProviderSettings::OpenAI(OpenAISettings {
                    url: "http://localhost:8080".to_string(),
                    api_key: String::new(),
                }),
                active_model: "llama3.2".to_string(),
                models: vec![ModelConfig {
                    name: "llama3.2".to_string(),
                    context_window: 32768,
                    vision: true,
                }],
                strip_images_after_view: false,
            },
            compaction: CompactionConfig {
                enabled: default_compaction_enabled(),
                mode: default_compaction_mode(),
                reserve_tokens: default_reserve_tokens(),
                keep_recent_tokens: default_keep_recent_tokens(),
                vcc: VccConfig::default(),
            },
            ui: UiConfig::default(),
            rtk: RtkConfig::default(),
            retry: RetryConfig::default(),
        }
    }
}

impl Config {
    /// Get the config directory path ($`XDG_CONFIG_HOME/respondami` or ~/.config/respondami).
    ///
    /// The fallback uses a real home lookup (never a literal, unexpanded `~`),
    /// so a system without `HOME` degrades to a relative `.config` rather than
    /// a path that is never expanded.
    #[must_use]
    pub fn config_dir() -> PathBuf {
        let base = dirs::config_dir()
            .or_else(|| dirs::home_dir().map(|home| home.join(".config")))
            .unwrap_or_else(|| PathBuf::from(".config"));
        base.join("respondami")
    }

    /// Get the config file path.
    #[must_use]
    pub fn config_path() -> PathBuf {
        Self::config_dir().join("config.json")
    }

    /// Load config from disk, creating defaults if not found.
    ///
    /// Reads `config.json` from the user config directory. If the file does not
    /// exist, creates it with default settings. Performs a migrate-on-load pass
    /// to add newer config sections (ui, rtk, retry) to legacy files.
    ///
    /// # Errors
    ///
    /// - File I/O errors when reading or writing the config file.
    /// - JSON parse errors if the config file is malformed.
    /// - Validation errors (see [`Self::validate`]).
    pub fn load() -> anyhow::Result<Self> {
        let path = Self::config_path();
        if !path.exists() {
            let config = Self::default();
            config.save()?;
            return Ok(config);
        }

        let content = fs::read_to_string(&path)
            .with_context(|| format!("Failed to read config {}", path.display()))?;

        // Check if the raw JSON has newer sections (for migrate-on-load)
        let raw: serde_json::Value = serde_json::from_str(&content)
            .with_context(|| format!("Failed to parse config {}", path.display()))?;
        let needs_migration = raw.get("ui").is_none()
            || raw.get("rtk").is_none()
            || raw.get("provider").and_then(|p| p.get("models")).is_none()
            || raw
                .get("provider")
                .and_then(|p| p.get("strip_images_after_view"))
                .is_none()
            || raw.get("compaction").and_then(|c| c.get("mode")).is_none()
            // A `type: llamacpp` config that still carries a `models` block is
            // legacy — it loads as `openai` and must be re-saved with the new
            // `type`.
            || {
                let p = raw.get("provider");
                p.and_then(|p| p.get("type"))
                    .and_then(serde_json::Value::as_str)
                    == Some("llamacpp")
                    && p.and_then(|p| p.get("models")).is_some()
            };

        let config: Self = serde_json::from_value(raw)
            .with_context(|| format!("Failed to parse config {}", path.display()))?;

        config.validate()?;

        // Migrate: write defaults if newer sections were absent
        if needs_migration {
            config.save()?;
        }
        Ok(config)
    }

    /// Save config to disk as pretty-printed JSON.
    ///
    /// Writes the serialized config to `config.json` in the user config
    /// directory, creating parent directories if needed.
    ///
    /// # Errors
    ///
    /// - Directory creation fails if the config path parent is not writable.
    /// - Serialization fails if the config contains non-serializable values.
    /// - File write fails due to I/O errors.
    pub fn save(&self) -> anyhow::Result<()> {
        let path = Self::config_path();
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).context("Failed to create config dir")?;
        }
        let content = serde_json::to_string_pretty(self).context("Failed to serialize config")?;
        fs::write(&path, content)
            .with_context(|| format!("Failed to write config {}", path.display()))?;
        Ok(())
    }

    /// Validate configuration values.
    ///
    /// Checks that the provider URL starts with `http://` or `https://`, that
    /// at least one model is configured, and that every model has a context
    /// window greater than zero. The `llamacpp` provider autodetects its
    /// models, so it only requires a valid URL (no model check).
    ///
    /// # Errors
    ///
    /// - Returns an error if no models are configured (openai only).
    /// - Returns an error if any model's `context_window` is zero (openai only).
    /// - Returns an error if the provider URL does not start with `http://` or `https://`.
    pub fn validate(&self) -> anyhow::Result<()> {
        let is_llamacpp = matches!(self.provider.settings, ProviderSettings::LlamaCpp(_));
        let url = match &self.provider.settings {
            ProviderSettings::OpenAI(s) => &s.url,
            ProviderSettings::LlamaCpp(s) => &s.url,
        };

        if !is_llamacpp {
            if self.provider.models.is_empty() {
                return Err(anyhow::anyhow!("at least one model must be configured"));
            }
            for model in &self.provider.models {
                if model.context_window == 0 {
                    return Err(anyhow::anyhow!(
                        "model '{}' context_window must be greater than 0",
                        model.name
                    ));
                }
            }
        }
        if !url.starts_with("http://") && !url.starts_with("https://") {
            return Err(anyhow::anyhow!(
                "provider url must start with http:// or https://"
            ));
        }
        Ok(())
    }
}
