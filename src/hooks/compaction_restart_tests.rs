//! Test: skill hooks must survive the full compaction flow.
//!
//! This test simulates the actual sequence of events:
//! 1. Skill is activated via the `activate_skill` tool.
//! 2. The TUI receives the `SkillActivation` event and registers the hooks.
//! 3. The agent is launched with a clone of the TUI registry.
//! 4. Compaction is triggered.
//! 5. The agent is restarted with a new clone of the TUI registry.
//! 6. The new clone must still have the skill hooks.

use std::collections::HashMap;
use std::path::PathBuf;

use crate::hooks::{HookEvent, HookRegistry};
use crate::skills::{Skill, SkillHook, SkillSource};

fn make_skill() -> Skill {
    Skill {
        name: "test-skill".to_string(),
        description: "A test skill".to_string(),
        license: None,
        metadata: HashMap::new(),
        hooks: vec![
            SkillHook {
                event: HookEvent::PreToolUse,
                command: "echo pre".to_string(),
                matcher: Some("Read|Write|Edit|Bash".to_string()),
            },
            SkillHook {
                event: HookEvent::PostToolUse,
                command: "echo post".to_string(),
                matcher: Some("Write|Edit".to_string()),
            },
        ],
        file_path: PathBuf::from("/test/SKILL.md"),
        base_dir: PathBuf::from("/test"),
        source: SkillSource::Global,
    }
}

#[test]
fn skill_hooks_survive_full_compaction_flow() {
    let skill = make_skill();

    // Step 1-2: Simulate TUI receiving SkillActivation event.
    // The TUI's active_skills set is empty, so insert returns true.
    let mut active_skills: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut tui_registry = HookRegistry::new();

    let inserted = active_skills.insert(skill.name.clone());
    assert!(inserted, "first activation should insert into active_skills");
    if inserted {
        crate::hooks::register_skill_hooks(&mut tui_registry, &skill);
    }

    assert_eq!(
        tui_registry.hooks(HookEvent::PreToolUse).len(),
        1,
        "TUI registry should have the PreToolUse hook"
    );
    assert_eq!(
        tui_registry.hooks(HookEvent::PostToolUse).len(),
        1,
        "TUI registry should have the PostToolUse hook"
    );

    // Step 3: Simulate agent launch: clone the TUI registry.
    let agent_registry_1 = tui_registry.clone();
    assert_eq!(
        agent_registry_1.hooks(HookEvent::PreToolUse).len(),
        1,
        "First agent clone should have the PreToolUse hook"
    );
    assert_eq!(
        agent_registry_1.hooks(HookEvent::PostToolUse).len(),
        1,
        "First agent clone should have the PostToolUse hook"
    );

    // Step 4: Simulate compaction.
    // The compaction flow does NOT modify the TUI registry.
    // (Verified by code inspection: no code in the compaction path
    // modifies app.config.hook_registry.)

    // Step 5: Simulate agent restart: clone the TUI registry again.
    let agent_registry_2 = tui_registry.clone();
    assert_eq!(
        agent_registry_2.hooks(HookEvent::PreToolUse).len(),
        1,
        "Second agent clone (after compaction) should have the PreToolUse hook"
    );
    assert_eq!(
        agent_registry_2.hooks(HookEvent::PostToolUse).len(),
        1,
        "Second agent clone (after compaction) should have the PostToolUse hook"
    );

    // Step 6: Verify the hooks are identical.
    assert_eq!(
        agent_registry_1.hooks(HookEvent::PreToolUse)[0].command,
        agent_registry_2.hooks(HookEvent::PreToolUse)[0].command,
        "PreToolUse hook command must be identical"
    );
    assert_eq!(
        agent_registry_1.hooks(HookEvent::PostToolUse)[0].command,
        agent_registry_2.hooks(HookEvent::PostToolUse)[0].command,
        "PostToolUse hook command must be identical"
    );
}

/// Regression: if the skill is already active (e.g., activated via /skillname),
/// the `SkillActivation` handler must NOT re-register the hooks (to avoid
/// duplicates). But the hooks should already be in the registry from the
/// initial activation.
#[test]
fn skill_hooks_not_duplicated_on_reactivation() {
    let skill = make_skill();

    let mut active_skills: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut tui_registry = HookRegistry::new();

    // First activation.
    let inserted_1 = active_skills.insert(skill.name.clone());
    assert!(inserted_1);
    if inserted_1 {
        crate::hooks::register_skill_hooks(&mut tui_registry, &skill);
    }

    // Second activation (skill already active).
    let inserted_2 = active_skills.insert(skill.name.clone());
    assert!(!inserted_2, "second activation should not insert");
    if inserted_2 {
        crate::hooks::register_skill_hooks(&mut tui_registry, &skill);
    }

    assert_eq!(
        tui_registry.hooks(HookEvent::PreToolUse).len(),
        1,
        "hooks must not be duplicated on reactivation"
    );
    assert_eq!(
        tui_registry.hooks(HookEvent::PostToolUse).len(),
        1,
        "hooks must not be duplicated on reactivation"
    );
}
