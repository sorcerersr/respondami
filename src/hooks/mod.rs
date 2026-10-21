//! Hooks system — shell scripts that execute at lifecycle points.
//!
//! Hooks fire synchronously inside the agent loop, blocking the agent's
//! execution until the hook completes. They can inject context (exit 0),
//! block actions (exit 2), or log non-blocking errors (other exit codes).
//!
//! Two locations for hooks:
//! 1. **Skill frontmatter** — YAML `hooks:` block in `SKILL.md`. Skill hooks
//!    fire only while the skill is active; they are registered on activation
//!    and re-derived from the skill files on session resume.
//! 2. **Directory-based** — shell scripts in `hooks/<hooktype>/` under
//!    `~/.config/respondami/hooks/` (global) or `.respondami/hooks/` (project)

mod executor;
mod loader;

#[doc(inline)]
pub use executor::{execute_hook, HookContext, HookResult};
#[doc(inline)]
pub use loader::{
    load_all_hooks, load_directory_hooks, Hook, HookEvent, HookSource, HookRegistry, ToolMatcher,
};
use crate::skills::{Skill, SkillSource};
use std::collections::HashSet;
use std::path::Path;

/// Register a skill's frontmatter hooks into the registry.
///
/// Hook names are tagged with the skill name (`<skill>:<script>`) for
/// deterministic ordering and TUI display. `SKILL_NAME`/`SKILL_DIR` env vars
/// are set from the skill at execution time. A matcher that fails to compile
/// is logged and the hook is registered as never-firing.
pub fn register_skill_hooks(registry: &mut HookRegistry, skill: &Skill) {
    for hook in &skill.hooks {
        let matcher = match &hook.matcher {
            None => ToolMatcher::All,
            Some(m) if m.trim().is_empty() || m == "*" => ToolMatcher::All,
            Some(m) => match regex::RegexBuilder::new(m).case_insensitive(true).build() {
                Ok(re) => ToolMatcher::Match(re),
                Err(e) => {
                    tracing::warn!(
                        skill = %skill.name,
                        matcher = %m,
                        error = %e,
                        "Skill hook matcher failed to compile — hook will never fire"
                    );
                    ToolMatcher::Never
                }
            },
        };
        registry.add(Hook {
            event: hook.event,
            name: format!("{}:{}", skill.name, derive_hook_name(&hook.command)),
            command: hook.command.clone(),
            source: skill.source.into(),
            skill_name: Some(skill.name.clone()),
            skill_dir: Some(skill.base_dir.clone()),
            matcher,
        });
    }
}

/// Rebuild the hook registry from directory hooks plus the skill hooks of
/// every active skill.
///
/// Used on session resume (restored `active_skills`), `/new` (no active
/// skills), and `reload_hooks` so skill hooks track the active set exactly.
#[must_use]
#[expect(
    clippy::implicit_hasher,
    reason = "skill-name set; the default hasher is the only sensible representation"
)]
pub fn rebuild_hook_registry(
    skills: &[Skill],
    active_skills: &HashSet<String>,
    config_dir: &Path,
    cwd: &Path,
) -> HookRegistry {
    let mut registry = load_all_hooks(config_dir, cwd);
    for name in active_skills {
        if let Some(skill) = skills.iter().find(|s| &s.name == name) {
            register_skill_hooks(&mut registry, skill);
        }
    }
    registry
}

/// Best-effort display name for a skill hook command.
///
/// Takes the file name of the first path-like token (e.g. `inject-plan.sh`
/// from `sh ${SKILL_DIR}/scripts/inject-plan.sh`); falls back to the full
/// command when no token contains a path separator.
pub(crate) fn derive_hook_name(command: &str) -> String {
    for token in command.split_whitespace() {
        let trimmed = token.trim_matches(|c| c == '"' || c == '\'');
        if trimmed.contains('/')
            && let Some(file_name) = Path::new(trimmed).file_name()
        {
            let name = file_name.to_string_lossy();
            if !name.is_empty() {
                return name.to_string();
            }
        }
    }
    command.to_string()
}

/// Format successful hook stdout into a context-injection suffix.
///
/// Each non-empty `(name, stdout)` pair becomes a visible
/// `[hook: <name>]` block appended after the host content (user message,
/// tool result, or compaction summary). Returns an empty string when no
/// hook produced output, so callers can append unconditionally.
#[must_use]
pub fn format_hook_context(outputs: &[(String, String)]) -> String {
    let mut suffix = String::new();
    for (name, stdout) in outputs {
        if !stdout.is_empty() {
            suffix.push_str(&format!("\n\n[hook: {name}]\n{stdout}"));
        }
    }
    suffix
}

impl From<SkillSource> for HookSource {
    fn from(value: SkillSource) -> Self {
        match value {
            SkillSource::Global => Self::Global,
            SkillSource::Project => Self::Project,
        }
    }
}

#[cfg(test)]
mod executor_tests;
#[cfg(test)]
mod loader_tests;
#[cfg(test)]
mod mod_tests;
