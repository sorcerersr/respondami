//! Tests for skill-hook registration and context formatting.

use std::collections::HashMap;
use std::path::PathBuf;

use super::*;
use crate::skills::SkillHook;
use tempfile::TempDir;

fn make_skill(name: &str, hooks: Vec<SkillHook>) -> Skill {
    Skill {
        name: name.to_string(),
        description: "test skill".to_string(),
        license: None,
        metadata: HashMap::new(),
        hooks,
        file_path: PathBuf::from(format!("/skills/{name}/SKILL.md")),
        base_dir: PathBuf::from(format!("/skills/{name}")),
        source: SkillSource::Global,
    }
}

// ---------------------------------------------------------------------------
// register_skill_hooks
// ---------------------------------------------------------------------------

#[test]
fn register_skill_hooks_noop_when_no_hooks() {
    let mut registry = HookRegistry::new();
    let skill = make_skill("plain", Vec::new());
    register_skill_hooks(&mut registry, &skill);
    assert_eq!(registry.total_count(), 0);
}

#[test]
fn register_skill_hooks_tags_name_with_skill() {
    let mut registry = HookRegistry::new();
    let skill = make_skill(
        "planning",
        vec![SkillHook {
            event: HookEvent::PreToolUse,
            command: "sh /skills/planning/scripts/inject-plan.sh".to_string(),
            matcher: Some("Write".to_string()),
        }],
    );
    register_skill_hooks(&mut registry, &skill);
    let hooks = registry.hooks(HookEvent::PreToolUse);
    assert_eq!(hooks.len(), 1);
    assert_eq!(hooks[0].name, "planning:inject-plan.sh");
    assert_eq!(hooks[0].skill_name, Some("planning".to_string()));
    assert_eq!(hooks[0].skill_dir, Some(PathBuf::from("/skills/planning")));
    assert_eq!(hooks[0].source, HookSource::Global);
}

#[test]
fn register_skill_hooks_matcher_all_when_none() {
    let mut registry = HookRegistry::new();
    let skill = make_skill(
        "s",
        vec![SkillHook {
            event: HookEvent::Stop,
            command: "echo done".to_string(),
            matcher: None,
        }],
    );
    register_skill_hooks(&mut registry, &skill);
    assert!(matches!(
        registry.hooks(HookEvent::Stop)[0].matcher,
        ToolMatcher::All
    ));
}

#[test]
fn register_skill_hooks_matcher_all_when_star() {
    let mut registry = HookRegistry::new();
    let skill = make_skill(
        "s",
        vec![SkillHook {
            event: HookEvent::PreToolUse,
            command: "echo x".to_string(),
            matcher: Some("*".to_string()),
        }],
    );
    register_skill_hooks(&mut registry, &skill);
    assert!(matches!(
        registry.hooks(HookEvent::PreToolUse)[0].matcher,
        ToolMatcher::All
    ));
}

#[test]
fn register_skill_hooks_matcher_never_when_invalid_regex() {
    let mut registry = HookRegistry::new();
    let skill = make_skill(
        "s",
        vec![SkillHook {
            event: HookEvent::PreToolUse,
            command: "echo x".to_string(),
            matcher: Some("[unclosed".to_string()),
        }],
    );
    register_skill_hooks(&mut registry, &skill);
    assert!(matches!(
        registry.hooks(HookEvent::PreToolUse)[0].matcher,
        ToolMatcher::Never
    ));
}

#[test]
fn register_skill_hooks_matcher_compiles_valid_regex() {
    let mut registry = HookRegistry::new();
    let skill = make_skill(
        "s",
        vec![SkillHook {
            event: HookEvent::PreToolUse,
            command: "echo x".to_string(),
            matcher: Some("Write|Edit".to_string()),
        }],
    );
    register_skill_hooks(&mut registry, &skill);
    let m = &registry.hooks(HookEvent::PreToolUse)[0].matcher;
    assert!(m.matches("Write"));
    assert!(m.matches("edit"));
    assert!(!m.matches("Read"));
}

// ---------------------------------------------------------------------------
// rebuild_hook_registry
// ---------------------------------------------------------------------------

#[test]
fn rebuild_hook_registry_includes_active_skill_hooks() {
    let config_dir = TempDir::new().unwrap();
    let cwd = TempDir::new().unwrap();
    let skill = make_skill(
        "active",
        vec![SkillHook {
            event: HookEvent::Stop,
            command: "echo bye".to_string(),
            matcher: None,
        }],
    );
    let inactive = make_skill(
        "inactive",
        vec![SkillHook {
            event: HookEvent::Stop,
            command: "echo no".to_string(),
            matcher: None,
        }],
    );
    let skills = vec![skill, inactive];
    let active: HashSet<String> = HashSet::from(["active".to_string()]);

    let registry = rebuild_hook_registry(&skills, &active, config_dir.path(), cwd.path());
    assert_eq!(registry.total_count(), 1);
    assert!(registry.has_hooks(HookEvent::Stop));
    assert_eq!(registry.hooks(HookEvent::Stop)[0].name, "active:echo bye");
}

#[test]
fn rebuild_hook_registry_empty_active_set() {
    let config_dir = TempDir::new().unwrap();
    let cwd = TempDir::new().unwrap();
    let skill = make_skill(
        "s",
        vec![SkillHook {
            event: HookEvent::Stop,
            command: "echo x".to_string(),
            matcher: None,
        }],
    );
    let active: HashSet<String> = HashSet::new();
    let registry = rebuild_hook_registry(&[skill], &active, config_dir.path(), cwd.path());
    assert_eq!(registry.total_count(), 0);
}

#[test]
fn rebuild_hook_registry_merges_directory_hooks() {
    let config_dir = TempDir::new().unwrap();
    let cwd = TempDir::new().unwrap();
    // A project directory hook.
    let pre_dir = cwd
        .path()
        .join(".respondami")
        .join("hooks")
        .join("PreToolUse");
    std::fs::create_dir_all(&pre_dir).unwrap();
    std::fs::write(pre_dir.join("dir-hook.sh"), "#!/bin/bash").unwrap();

    let skill = make_skill(
        "s",
        vec![SkillHook {
            event: HookEvent::Stop,
            command: "echo x".to_string(),
            matcher: None,
        }],
    );
    let active: HashSet<String> = HashSet::from(["s".to_string()]);
    let registry = rebuild_hook_registry(&[skill], &active, config_dir.path(), cwd.path());
    assert_eq!(registry.total_count(), 2);
    assert!(registry.has_hooks(HookEvent::PreToolUse));
    assert!(registry.has_hooks(HookEvent::Stop));
}

// ---------------------------------------------------------------------------
// derive_hook_name
// ---------------------------------------------------------------------------

#[test]
fn derive_hook_name_from_path_token() {
    assert_eq!(
        derive_hook_name("sh /skills/planning/scripts/inject-plan.sh"),
        "inject-plan.sh"
    );
}

#[test]
fn derive_hook_name_strips_quotes() {
    assert_eq!(
        derive_hook_name("sh '${SKILL_DIR}/scripts/run.sh'"),
        "run.sh"
    );
}

#[test]
fn derive_hook_name_falls_back_to_full_command() {
    assert_eq!(derive_hook_name("echo hello"), "echo hello");
    assert_eq!(derive_hook_name("ls -la"), "ls -la");
}

#[test]
fn derive_hook_name_ignores_env_var_tokens() {
    // `${SKILL_DIR}` has no path separator, so the next path-like token wins.
    assert_eq!(derive_hook_name("bash ${SKILL_DIR}/a/b/real.sh"), "real.sh");
}

#[test]
fn derive_hook_name_skips_env_assignment_tokens() {
    // The planning-with-files Stop hook command, verbatim: the first token
    // is a variable assignment whose default value contains a path; the
    // script file in the `-f` test must win instead.
    let cmd = r#"SKILL_DIR="${SKILL_DIR:-/home/dev/.config/respondami/skills/planning-with-files}"; [ -f "${SKILL_DIR}/scripts/gate-stop.sh" ] && sh "${SKILL_DIR}/scripts/gate-stop.sh"; exit 0"#;
    assert_eq!(derive_hook_name(cmd), "gate-stop.sh");
}

#[test]
fn derive_hook_name_skips_simple_assignment() {
    assert_eq!(derive_hook_name("FOO=bar sh /a/b/c.sh"), "c.sh");
}

#[test]
fn derive_hook_name_assignment_only_falls_back() {
    // No non-assignment path-like token → full-command fallback.
    assert_eq!(derive_hook_name("SKILL_DIR=/a/b/c"), "SKILL_DIR=/a/b/c");
}

// ---------------------------------------------------------------------------
// format_hook_context
// ---------------------------------------------------------------------------

#[test]
fn format_hook_context_empty() {
    assert_eq!(format_hook_context(&[]), "");
}

#[test]
fn format_hook_context_skips_empty_stdout() {
    let out = format_hook_context(&[
        ("a".to_string(), String::new()),
        ("b".to_string(), String::new()),
    ]);
    assert_eq!(out, "");
}

#[test]
fn format_hook_context_single() {
    let out = format_hook_context(&[("a".to_string(), "hello".to_string())]);
    assert_eq!(out, "\n\n[hook: a]\nhello");
}

#[test]
fn format_hook_context_multiple() {
    let out = format_hook_context(&[
        ("a".to_string(), "one".to_string()),
        ("b".to_string(), "two".to_string()),
    ]);
    assert_eq!(out, "\n\n[hook: a]\none\n\n[hook: b]\ntwo");
}

// ---------------------------------------------------------------------------
// run_tool_hooks: stderr carried to the TUI event
// ---------------------------------------------------------------------------

#[tokio::test]
async fn run_tool_hooks_carries_real_stderr() {
    let (tx, mut rx) = tokio::sync::mpsc::channel::<crate::tui::AgentEvent>(16);
    let mut registry = HookRegistry::new();
    registry.add(Hook {
        event: HookEvent::PreToolUse,
        name: "err.sh".to_string(),
        command: "echo oops >&2; exit 0".to_string(),
        script: None,
        source: HookSource::Project,
        skill_name: None,
        skill_dir: None,
        matcher: ToolMatcher::All,
    });

    let (blocked, error, results) = run_tool_hooks(
        &tx,
        &registry,
        HookEvent::PreToolUse,
        std::path::Path::new("."),
        "bash",
        &serde_json::json!({"command": "ls"}),
        None,
    )
    .await;

    assert!(!blocked);
    assert_eq!(error, "");
    assert_eq!(results.len(), 1);
    assert!(results[0].1.success());

    let event = rx.recv().await.expect("hook message on channel");
    match event {
        crate::tui::AgentEvent::HookMessage {
            success, stderr, ..
        } => {
            assert!(success);
            assert!(
                stderr.contains("oops"),
                "stderr must be carried to the TUI event, got: {stderr:?}"
            );
        }
        other => panic!("expected HookMessage, got {other:?}"),
    }
}

// ---------------------------------------------------------------------------
// SkillSource -> HookSource
// ---------------------------------------------------------------------------

#[test]
fn skill_source_to_hook_source() {
    assert_eq!(HookSource::from(SkillSource::Global), HookSource::Global);
    assert_eq!(HookSource::from(SkillSource::Project), HookSource::Project);
}
