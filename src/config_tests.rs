use crate::config::{
    CompactionConfig, CompactionMode, Config, ProviderConfig, RtkConfig, UiConfig, VccConfig,
};
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
    assert!(config.vision, "vision defaults to true");
    assert!(!config.strip_images_after_view, "strip_images_after_view defaults to false");
    let ProviderSettings::LlamaCpp(s) = &config.settings;
    assert_eq!(s.url, "http://localhost:8080");
    assert_eq!(s.api_key, "");
}

#[test]
fn provider_config_strip_images_after_view_true() {
    let json = r#"{
    "type": "llamacpp",
    "url": "http://example.com:8080",
    "model": "my-model",
    "context_window": 4096,
    "strip_images_after_view": true
}"#;
    let config: ProviderConfig = serde_json::from_str(json).unwrap();
    assert!(
        config.strip_images_after_view,
        "strip_images_after_view: true must be honored"
    );
}

#[test]
fn provider_config_vision_false() {
    let json = r#"{
    "type": "llamacpp",
    "url": "http://example.com:8080",
    "model": "my-model",
    "context_window": 4096,
    "vision": false
}"#;
    let config: ProviderConfig = serde_json::from_str(json).unwrap();
    assert!(!config.vision, "vision: false must be honored");
}

#[test]
fn config_top_level_vision_ignored() {
    // Legacy top-level `vision` is not a valid config location; it must be
    // ignored in favor of `provider.vision` (default true).
    let json = r#"{
    "provider": {
        "type": "llamacpp",
        "url": "http://localhost:8080",
        "model": "llama3.2",
        "context_window": 32768
    },
    "compaction": { "enabled": true, "reserve_tokens": 16384, "keep_recent_tokens": 16384 },
    "vision": false
}"#;
    let config: Config = serde_json::from_str(json).unwrap();
    assert!(config.provider.vision, "top-level vision must be ignored");
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
        vision: false,
        strip_images_after_view: true,
    };
    let json = serde_json::to_string(&config).unwrap();
    let decoded: ProviderConfig = serde_json::from_str(&json).unwrap();
    assert_eq!(decoded.model, config.model);
    assert_eq!(decoded.context_window, config.context_window);
    assert_eq!(decoded.provider_type(), config.provider_type());
    assert!(!decoded.vision, "vision must round-trip inside the provider object");
    assert!(
        decoded.strip_images_after_view,
        "strip_images_after_view must round-trip inside the provider object"
    );
    assert!(
        json.contains("\"vision\":false"),
        "vision must serialize inside the provider object: {json}"
    );
    assert!(
        json.contains("\"strip_images_after_view\":true"),
        "strip_images_after_view must serialize inside the provider object: {json}"
    );
}

// ---------------------------------------------------------------------------
// Config defaults
// ---------------------------------------------------------------------------

#[test]
fn config_default_values() {
    let config = Config::default();
    assert_eq!(config.provider.model, "llama3.2");
    assert_eq!(config.provider.context_window, 32768);
    assert!(config.provider.vision);
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

#[test]
fn compaction_config_mode_defaults_to_vcc() {
    // Legacy config without a `mode` key must load with the new default.
    let json = r#"{"enabled": true, "reserve_tokens": 16384, "keep_recent_tokens": 16384}"#;
    let config: CompactionConfig = serde_json::from_str(json).unwrap();
    assert_eq!(config.mode, CompactionMode::Vcc, "mode defaults to vcc on upgrade");
    assert!(config.vcc.smart_keep_tail, "smart_keep_tail defaults to true");
    assert_eq!(config.vcc.brief_budget_tokens, 1100, "brief_budget_tokens defaults to 1100");
}

#[test]
fn compaction_config_mode_llm_honored() {
    let json = r#"{"mode": "llm"}"#;
    let config: CompactionConfig = serde_json::from_str(json).unwrap();
    assert_eq!(config.mode, CompactionMode::Llm);
}

#[test]
fn compaction_config_mode_vcc_honored() {
    let json = r#"{"mode": "vcc"}"#;
    let config: CompactionConfig = serde_json::from_str(json).unwrap();
    assert_eq!(config.mode, CompactionMode::Vcc);
}

#[test]
fn compaction_config_unknown_mode_rejects() {
    let json = r#"{"mode": "bogus"}"#;
    let result: Result<CompactionConfig, _> = serde_json::from_str(json);
    result.unwrap_err();
}

#[test]
fn vcc_config_custom_values() {
    let json = r#"{"smart_keep_tail": false, "brief_budget_tokens": 2000}"#;
    let vcc: VccConfig = serde_json::from_str(json).unwrap();
    assert!(!vcc.smart_keep_tail);
    assert_eq!(vcc.brief_budget_tokens, 2000);
}

#[test]
fn vcc_config_missing_uses_defaults() {
    let json = r"{}";
    let vcc: VccConfig = serde_json::from_str(json).unwrap();
    assert!(vcc.smart_keep_tail);
    assert_eq!(vcc.brief_budget_tokens, 1100);
}

#[test]
fn compaction_config_vcc_roundtrip() {
    let config = CompactionConfig {
        enabled: true,
        mode: CompactionMode::Vcc,
        reserve_tokens: 20000,
        keep_recent_tokens: 16384,
        vcc: VccConfig {
            smart_keep_tail: false,
            brief_budget_tokens: 1500,
        },
    };
    let json = serde_json::to_string(&config).unwrap();
    let decoded: CompactionConfig = serde_json::from_str(&json).unwrap();
    assert_eq!(decoded.mode, CompactionMode::Vcc);
    assert!(!decoded.vcc.smart_keep_tail);
    assert_eq!(decoded.vcc.brief_budget_tokens, 1500);
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
    assert_eq!(config.ui.file_always_visible, vec!["docs/plans/", "docs/reviews/"]);
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

#[test]
fn ui_config_file_always_visible_serde_default() {
    let json = r"{}";
    let ui: UiConfig = serde_json::from_str(json).unwrap();
    assert_eq!(ui.file_always_visible, vec!["docs/plans/", "docs/reviews/"]);
}

#[test]
fn ui_config_file_always_visible_replaces_default_wholesale() {
    let json = r#"{"file_always_visible": ["notes/private/"]}"#;
    let ui: UiConfig = serde_json::from_str(json).unwrap();
    assert_eq!(ui.file_always_visible, vec!["notes/private/"]);

    let json = r#"{"file_always_visible": []}"#;
    let ui: UiConfig = serde_json::from_str(json).unwrap();
    assert_eq!(ui.file_always_visible, [] as [String; 0]);
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
