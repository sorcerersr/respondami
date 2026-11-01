use crate::config::{
    CompactionConfig, CompactionMode, Config, ProviderConfig, RtkConfig, UiConfig, VccConfig,
};
use crate::provider::{OpenAISettings, ProviderSettings};
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
    assert_eq!(config.active_model, "my-model");
    assert_eq!(config.active_context_window(), 4096);
    // Legacy `llamacpp` + model block migrates to `openai`.
    assert_eq!(config.provider_type(), "openai");
}

#[test]
fn provider_config_legacy_no_type_field() {
    // Legacy config without "type" field migrates to openai
    let json = r#"{
    "url": "http://localhost:9000",
    "model": "llama3.2",
    "context_window": 8192
}"#;
    let config: ProviderConfig = serde_json::from_str(json).unwrap();
    assert_eq!(config.provider_type(), "openai");
    assert_eq!(config.active_model, "llama3.2");
    assert_eq!(config.active_context_window(), 8192);
}

#[test]
fn provider_config_llamacpp_autodetect() {
    // New `llamacpp` provider: no model block, no active_model.
    let json = r#"{
    "type": "llamacpp",
    "url": "http://192.168.178.68:8080",
    "api_key": "1234"
}"#;
    let config: ProviderConfig = serde_json::from_str(json).unwrap();
    assert_eq!(config.provider_type(), "llamacpp");
    assert!(config.models.is_empty(), "llamacpp has no configured models");
    assert_eq!(config.active_model, "", "llamacpp active model resolved at startup");
    let ProviderSettings::LlamaCpp(s) = &config.settings else {
        panic!("expected LlamaCpp settings");
    };
    assert_eq!(s.url, "http://192.168.178.68:8080");
    assert_eq!(s.api_key, "1234");
}

#[test]
fn provider_config_llamacpp_roundtrip_skips_models() {
    // `llamacpp` serializes without active_model/models.
    let config = ProviderConfig {
        settings: ProviderSettings::LlamaCpp(crate::provider::LlamaCppSettings {
            url: "http://192.168.178.68:8080".to_string(),
            api_key: "1234".to_string(),
        }),
        models: Vec::new(),
        active_model: String::new(),
        strip_images_after_view: false,
    };
    let json = serde_json::to_string(&config).unwrap();
    assert!(json.contains("\"type\":\"llamacpp\""), "{json}");
    assert!(!json.contains("\"models\""), "llamacpp must not serialize models: {json}");
    assert!(!json.contains("\"active_model\""), "llamacpp must not serialize active_model: {json}");
    let decoded: ProviderConfig = serde_json::from_str(&json).unwrap();
    assert_eq!(decoded.provider_type(), "llamacpp");
    assert_eq!(decoded.models.len(), 0);
}

#[test]
fn provider_config_openai_explicit_type() {
    let json = r#"{
    "type": "openai",
    "url": "http://example.com:8080",
    "api_key": "secret",
    "active_model": "my-model",
    "models": [ { "name": "my-model", "context_window": 4096, "vision": false } ]
}"#;
    let config: ProviderConfig = serde_json::from_str(json).unwrap();
    assert_eq!(config.provider_type(), "openai");
    assert_eq!(config.active_model, "my-model");
    assert_eq!(config.active_context_window(), 4096);
    assert!(!config.active_vision());
}

#[test]
fn provider_config_defaults() {
    // Minimal config — all fields should default
    let json = r"{}";
    let config: ProviderConfig = serde_json::from_str(json).unwrap();
    assert_eq!(config.active_model, "llama3.2");
    assert_eq!(config.active_context_window(), 32768);
    assert!(config.active_vision(), "vision defaults to true");
    assert!(
        !config.strip_images_after_view,
        "strip_images_after_view defaults to false"
    );
    let ProviderSettings::OpenAI(s) = &config.settings else {
        panic!("expected OpenAI settings");
    };
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
    assert!(!config.active_vision(), "vision: false must be honored");
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
    assert!(
        config.provider.active_vision(),
        "top-level vision must be ignored"
    );
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
        settings: ProviderSettings::OpenAI(OpenAISettings {
            url: "http://localhost:8080".to_string(),
            api_key: "key123".to_string(),
        }),
        models: vec![crate::config::ModelConfig {
            name: "my-model".to_string(),
            context_window: 4096,
            vision: false,
        }],
        active_model: "my-model".to_string(),
        strip_images_after_view: true,
    };
    let json = serde_json::to_string(&config).unwrap();
    let decoded: ProviderConfig = serde_json::from_str(&json).unwrap();
    assert_eq!(decoded.active_model, config.active_model);
    assert_eq!(
        decoded.active_context_window(),
        config.active_context_window()
    );
    assert_eq!(decoded.provider_type(), config.provider_type());
    assert!(
        !decoded.active_vision(),
        "vision must round-trip inside the provider object"
    );
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
    assert_eq!(config.provider.active_model, "llama3.2");
    assert_eq!(config.provider.active_context_window(), 32768);
    assert!(config.provider.active_vision());
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
    config.provider.models[0].context_window = 0;
    assert!(config.validate().is_err());
}

#[test]
fn config_validate_invalid_url() {
    let mut config = Config::default();
    let ProviderSettings::OpenAI(s) = &mut config.provider.settings else {
        panic!("expected OpenAI settings");
    };
    s.url = "not-a-url".to_string();
    assert!(config.validate().is_err());
}

#[test]
fn config_validate_https_url_ok() {
    let mut config = Config::default();
    let ProviderSettings::OpenAI(s) = &mut config.provider.settings else {
        panic!("expected OpenAI settings");
    };
    s.url = "https://example.com".to_string();
    config.validate().unwrap();
}

#[test]
fn config_validate_llamacpp_no_models_ok() {
    // `llamacpp` requires only a valid URL; no models needed.
    let mut config = Config::default();
    config.provider.settings =
        ProviderSettings::LlamaCpp(crate::provider::LlamaCppSettings {
            url: "http://192.168.178.68:8080".to_string(),
            api_key: "1234".to_string(),
        });
    config.provider.models.clear();
    config.provider.active_model.clear();
    config.validate().unwrap();
}

#[test]
fn config_validate_llamacpp_bad_url_rejects() {
    let mut config = Config::default();
    config.provider.settings =
        ProviderSettings::LlamaCpp(crate::provider::LlamaCppSettings {
            url: "not-a-url".to_string(),
            api_key: String::new(),
        });
    assert!(config.validate().is_err());
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
    assert_eq!(
        config.mode,
        CompactionMode::Vcc,
        "mode defaults to vcc on upgrade"
    );
    assert!(
        config.vcc.smart_keep_tail,
        "smart_keep_tail defaults to true"
    );
    assert_eq!(
        config.vcc.brief_budget_tokens, 1100,
        "brief_budget_tokens defaults to 1100"
    );
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
    assert_eq!(decoded.provider.active_model, config.provider.active_model);
    assert_eq!(decoded.compaction.enabled, config.compaction.enabled);
}

// ---------------------------------------------------------------------------
// UiConfig
// ---------------------------------------------------------------------------

#[test]
fn config_default_ui() {
    let config = Config::default();
    assert!(matches!(
        config.ui.thinking_display,
        ThinkingDisplay::Collapsed
    ));
    assert_eq!(config.ui.thinking_max_lines, 5);
    assert!(config.ui.tool_output_expanded);
    assert!(config.ui.file_show_hidden);
    assert_eq!(
        config.ui.file_always_visible,
        vec!["docs/plans/", "docs/reviews/"]
    );
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
