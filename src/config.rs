//! Application configuration — loading, validation, and schema migration.
//!
//! Reads `config.json` from `~/.config/respondami/` and `.respondami/config.json`
//! (project-level overrides). Handles legacy config migration, provider settings,
//! compaction thresholds, UI display modes, and RTK integration settings.

use anyhow::Context;
use serde::ser::Error;
use serde::{Deserialize, Deserializer, Serialize};
use std::fs;
use std::path::PathBuf;

use crate::provider::{LlamaCppSettings, ProviderSettings};
use crate::tui::{HookDisplay, ThinkingDisplay};

/// Provider configuration for connecting to an LLM.
///
/// Uses an internally-tagged `settings` enum (`type` discriminator in JSON).
/// Legacy configs (without `type`) are auto-migrated to `llamacpp` on load.
#[derive(Debug, Clone)]
pub struct ProviderConfig {
    /// Provider-specific settings (url, `api_key`, etc.).
    pub settings: ProviderSettings,
    pub model: String,
    pub context_window: u32,
}

impl ProviderConfig {
    /// Get the provider type name.
    #[must_use]
    pub fn provider_type(&self) -> &str {
        match &self.settings {
            ProviderSettings::LlamaCpp(_) => "llamacpp",
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
        let model = obj
            .get("model")
            .and_then(|v| v.as_str())
            .unwrap_or("llama3.2")
            .to_string();
        let context_window = obj
            .get("context_window")
            .and_then(serde_json::Value::as_u64)
            .map_or(32768, |v| v as u32);

        // Build settings based on `type` field (or default to llamacpp for legacy)
        let settings = match obj
            .get("type")
            .and_then(|v| v.as_str())
        {
            Some("anthropic") => {
                // Future: parse AnthropicSettings
                return Err(D::Error::custom(
                    "anthropic provider not yet implemented",
                ));
            }
            Some("llamacpp") | None => {
                ProviderSettings::LlamaCpp(LlamaCppSettings { url, api_key })
            }
            Some(other) => {
                return Err(D::Error::custom(format!(
                    "unknown provider type: {other}"
                )));
            }
        };

        Ok(ProviderConfig {
            settings,
            model,
            context_window,
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
        map.insert("type".to_string(), serde_json::Value::String(self.provider_type().to_string()));

        // Add provider-specific fields
        match &self.settings {
            ProviderSettings::LlamaCpp(s) => {
                let s_val = serde_json::to_value(s).map_err(S::Error::custom)?;
                if let Some(obj) = s_val.as_object() {
                    for (k, v) in obj {
                        map.insert(k.clone(), v.clone());
                    }
                }
            }
        }

        map.insert("model".to_string(), serde_json::Value::String(self.model.clone()));
        map.insert(
            "context_window".to_string(),
            serde_json::Value::Number(self.context_window.into()),
        );

        map.serialize(serializer)
    }
}

/// Compaction settings.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompactionConfig {
    #[serde(default = "default_compaction_enabled")]
    pub enabled: bool,
    #[serde(default = "default_reserve_tokens")]
    pub reserve_tokens: u32,
    #[serde(default = "default_keep_recent_tokens")]
    pub keep_recent_tokens: u32,
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

impl Default for UiConfig {
    fn default() -> Self {
        Self {
            thinking_display: ThinkingDisplay::default(),
            thinking_max_lines: default_thinking_max_lines(),
            tool_output_expanded: default_tool_output_expanded(),
            hook_display: HookDisplay::default(),
            file_show_hidden: default_file_show_hidden(),
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
                settings: ProviderSettings::LlamaCpp(LlamaCppSettings {
                    url: "http://localhost:8080".to_string(),
                    api_key: String::new(),
                }),
                model: "llama3.2".to_string(),
                context_window: 32768,
            },
            compaction: CompactionConfig {
                enabled: default_compaction_enabled(),
                reserve_tokens: default_reserve_tokens(),
                keep_recent_tokens: default_keep_recent_tokens(),
            },
            ui: UiConfig::default(),
            rtk: RtkConfig::default(),
            retry: RetryConfig::default(),
        }
    }
}

impl Config {
    /// Get the config directory path ($`XDG_CONFIG_HOME/respondami` or ~/.config/respondami).
    #[must_use]
    pub fn config_dir() -> PathBuf {
        dirs::config_dir()
            .unwrap_or_else(|| PathBuf::from("~/.config"))
            .join("respondami")
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
        let needs_migration = raw.get("ui").is_none() || raw.get("rtk").is_none();

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
            fs::create_dir_all(parent)
                .context("Failed to create config dir")?;
        }
        let content = serde_json::to_string_pretty(self)
            .context("Failed to serialize config")?;
        fs::write(&path, content)
            .with_context(|| format!("Failed to write config {}", path.display()))?;
        Ok(())
    }

    /// Validate configuration values.
    ///
    /// Checks that the provider URL starts with `http://` or `https://` and
    /// that `context_window` is greater than zero.
    ///
    /// # Errors
    ///
    /// - Returns an error if `context_window` is zero.
    /// - Returns an error if the provider URL does not start with `http://` or `https://`.
    pub fn validate(&self) -> anyhow::Result<()> {
        let url = match &self.provider.settings {
            ProviderSettings::LlamaCpp(s) => &s.url,
        };

        if self.provider.context_window == 0 {
            return Err(anyhow::anyhow!(
                "context_window must be greater than 0"
            ));
        }
        if !url.starts_with("http://") && !url.starts_with("https://") {
            return Err(anyhow::anyhow!(
                "provider url must start with http:// or https://"
            ));
        }
        Ok(())
    }
}


