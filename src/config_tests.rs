use crate::config::{Config, CompactionConfig, ProviderConfig, RtkConfig, UiConfig};
use crate::provider::{LlamaCppSettings, ProviderSettings};
use crate::tui::ThinkingDisplay;

// ---------------------------------------------------------------------------
// ProviderConfig deserialization
// ---------------------------------------------------------------------------

#[test]
fn provider_config_full() {
    let json = r#"{
    "type": "llamacpp",
    "url": "http://example.com:8080",
    "api_key": "secret",
    "model": "my-model",
    "context_window": 4096
}"#;
    let config: ProviderConfig = serde_json::from_str(json).unwrap();
    assert_eq!(config.model, "my-model");
    assert_eq!(config.context_window, 4096);
    assert_eq!(config.provider_type(), "llamacpp");
}

#[test]
fn provider_config_legacy_no_type_field() {
    // Legacy config without "type" field should default to llamacpp
    let json = r#"{
    "url": "http://localhost:9000",
    "model": "llama3.2",
    "context_window": 8192
}"#;
    let config: ProviderConfig = serde_json::from_str(json).unwrap();
    assert_eq!(config.provider_type(), "llamacpp");
    assert_eq!(config.model, "llama3.2");
    assert_eq!(config.context_window, 8192);
}

#[test]
fn provider_config_defaults() {
    // Minimal config — all fields should default
    let json = r"{}";
    let config: ProviderConfig = serde_json::from_str(json).unwrap();
    assert_eq!(config.model, "llama3.2");
    assert_eq!(config.context_window, 32768);
    let ProviderSettings::LlamaCpp(s) = &config.settings;
    assert_eq!(s.url, "http://localhost:8080");
    assert_eq!(s.api_key, "");
}

#[test]
fn provider_config_unknown_type_rejects() {
    let json = r#"{
    "type": "unknown_provider",
    "model": "test",
    "context_window": 4096
}"#;
    let result: Result<ProviderConfig, _> = serde_json::from_str(json);
    result.unwrap_err();
}

// ---------------------------------------------------------------------------
// ProviderConfig serialization
// ---------------------------------------------------------------------------

#[test]
fn provider_config_roundtrip() {
    let config = ProviderConfig {
        settings: ProviderSettings::LlamaCpp(LlamaCppSettings {
            url: "http://localhost:8080".to_string(),
            api_key: "key123".to_string(),
        }),
        model: "my-model".to_string(),
        context_window: 4096,
    };
    let json = serde_json::to_string(&config).unwrap();
    let decoded: ProviderConfig = serde_json::from_str(&json).unwrap();
    assert_eq!(decoded.model, config.model);
    assert_eq!(decoded.context_window, config.context_window);
    assert_eq!(decoded.provider_type(), config.provider_type());
}

// ---------------------------------------------------------------------------
// Config defaults
// ---------------------------------------------------------------------------

#[test]
fn config_default_values() {
    let config = Config::default();
    assert_eq!(config.provider.model, "llama3.2");
    assert_eq!(config.provider.context_window, 32768);
    assert!(config.compaction.enabled);
    assert_eq!(config.compaction.reserve_tokens, 16384);
    assert_eq!(config.compaction.keep_recent_tokens, 16384);
}

#[test]
fn config_validate_valid() {
    let config = Config::default();
    config.validate().unwrap();
}

#[test]
fn config_validate_zero_context_window() {
    let mut config = Config::default();
    config.provider.context_window = 0;
    assert!(config.validate().is_err());
}

#[test]
fn config_validate_invalid_url() {
    let mut config = Config::default();
    let ProviderSettings::LlamaCpp(s) = &mut config.provider.settings;
    s.url = "not-a-url".to_string();
    assert!(config.validate().is_err());
}

#[test]
fn config_validate_https_url_ok() {
    let mut config = Config::default();
    let ProviderSettings::LlamaCpp(s) = &mut config.provider.settings;
    s.url = "https://example.com".to_string();
    config.validate().unwrap();
}

// ---------------------------------------------------------------------------
// CompactionConfig defaults
// ---------------------------------------------------------------------------

#[test]
fn compaction_config_defaults() {
    let json = r"{}";
    let config: CompactionConfig = serde_json::from_str(json).unwrap();
    assert!(config.enabled);
    assert_eq!(config.reserve_tokens, 16384);
    assert_eq!(config.keep_recent_tokens, 16384);
}

#[test]
fn compaction_config_custom() {
    let json = r#"{"enabled": false, "reserve_tokens": 4096}"#;
    let config: CompactionConfig = serde_json::from_str(json).unwrap();
    assert!(!config.enabled);
    assert_eq!(config.reserve_tokens, 4096);
    // keep_recent_tokens should use default
    assert_eq!(config.keep_recent_tokens, 16384);
}

// ---------------------------------------------------------------------------
// Full Config serialization
// ---------------------------------------------------------------------------

#[test]
fn config_full_roundtrip() {
    let config = Config::default();
    let json = serde_json::to_string(&config).unwrap();
    let decoded: Config = serde_json::from_str(&json).unwrap();
    assert_eq!(decoded.provider.model, config.provider.model);
    assert_eq!(decoded.compaction.enabled, config.compaction.enabled);
}

// ---------------------------------------------------------------------------
// UiConfig
// ---------------------------------------------------------------------------

#[test]
fn config_default_ui() {
    let config = Config::default();
    assert!(matches!(config.ui.thinking_display, ThinkingDisplay::Collapsed));
    assert_eq!(config.ui.thinking_max_lines, 5);
    assert!(config.ui.tool_output_expanded);
    assert!(config.ui.file_show_hidden);
}

#[test]
fn ui_config_custom_values() {
    let json = r#"{
    "thinking_display": "hidden",
    "thinking_max_lines": 3,
    "tool_output_expanded": true
}"#;
    let ui: UiConfig = serde_json::from_str(json).unwrap();
    assert!(matches!(ui.thinking_display, ThinkingDisplay::Hidden));
    assert_eq!(ui.thinking_max_lines, 3);
    assert!(ui.tool_output_expanded);
}

#[test]
fn ui_config_missing_uses_defaults() {
    let json = r"{}";
    let ui: UiConfig = serde_json::from_str(json).unwrap();
    assert!(matches!(ui.thinking_display, ThinkingDisplay::Collapsed));
    assert_eq!(ui.thinking_max_lines, 5);
    assert!(ui.tool_output_expanded);
}

#[test]
fn config_full_roundtrip_with_ui() {
    let config = Config::default();
    let json = serde_json::to_string(&config).unwrap();
    let decoded: Config = serde_json::from_str(&json).unwrap();
    assert_eq!(decoded.ui.thinking_display, config.ui.thinking_display);
    assert_eq!(decoded.ui.thinking_max_lines, config.ui.thinking_max_lines);
}

// ---------------------------------------------------------------------------
// RtkConfig
// ---------------------------------------------------------------------------

#[test]
fn rtk_config_default_values() {
    let config = RtkConfig::default();
    assert!(config.enabled);
}

#[test]
fn rtk_config_custom_values() {
    let json = r#"{
    "enabled": false
}"#;
    let config: RtkConfig = serde_json::from_str(json).unwrap();
    assert!(!config.enabled);
}

#[test]
fn rtk_config_missing_uses_defaults() {
    let json = r"{}";
    let config: RtkConfig = serde_json::from_str(json).unwrap();
    assert!(config.enabled);
}

#[test]
fn config_default_includes_rtk() {
    let config = Config::default();
    assert!(config.rtk.enabled);
}

#[test]
fn config_full_roundtrip_with_rtk() {
    let config = Config::default();
    let json = serde_json::to_string(&config).unwrap();
    let decoded: Config = serde_json::from_str(&json).unwrap();
    assert_eq!(decoded.rtk.enabled, config.rtk.enabled);
}

#[test]
fn config_without_rtk_section_loads_defaults() {
    // Simulates legacy config without rtk section
    let json = r#"{
    "provider": {
        "type": "llamacpp",
        "url": "http://localhost:8080",
        "model": "llama3.2",
        "context_window": 32768
    },
    "compaction": {
        "enabled": true,
        "reserve_tokens": 8192,
        "keep_recent_tokens": 16384
    },
    "ui": {
        "thinking_display": "collapsed",
        "thinking_max_lines": 5
    }
}"#;
    let config: Config = serde_json::from_str(json).unwrap();
    assert!(config.rtk.enabled);
}
