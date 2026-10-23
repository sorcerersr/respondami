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
use crate::tui::{AgentEvent, App, ChatMessage, HookDisplay};
use std::collections::HashSet;
use std::path::Path;
use tokio::sync::mpsc;

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
/// from `sh ${SKILL_DIR}/scripts/inject-plan.sh`); shell variable
/// assignment tokens (e.g. `SKILL_DIR="${SKILL_DIR:-/...}";`) are skipped so
/// a default path inside an assignment can't win; falls back to the full
/// command when no token contains a path separator.
pub(crate) fn derive_hook_name(command: &str) -> String {
    for token in command.split_whitespace() {
        if is_env_assignment(token) {
            continue;
        }
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

/// Whether a whitespace token is a shell variable assignment (`IDENT=...`).
///
/// Used by [`derive_hook_name`] to skip assignments like
/// `SKILL_DIR="${SKILL_DIR:-/...}";` whose value may contain a path.
fn is_env_assignment(token: &str) -> bool {
    token
        .split_once('=')
        .is_some_and(|(key, _)| {
            !key.is_empty()
                && key.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
        })
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

/// Run all hooks for `event` on the TUI side (app-based call sites).
///
/// Pushes one `ChatMessage::Hook` per executed hook, suppressed when
/// `hook_display` is `Hidden` (uniform across every call site). A blocking
/// hook (exit 2) stops the run; its stderr is returned as the error.
///
/// Returns `(blocked, blocked_error, outputs)` where `outputs` is the
/// `(name, stdout)` of each successful hook, in registry order.
pub(crate) async fn run_tui_hooks(
    app: &mut App,
    event: HookEvent,
    prompt: Option<String>,
) -> (bool, String, Vec<(String, String)>) {
    let mut blocked = false;
    let mut blocked_error = String::new();
    let mut outputs: Vec<(String, String)> = Vec::new();
    for hook in app.config.hook_registry.hooks(event) {
        let context = HookContext {
            event,
            hook_name: hook.name.clone(),
            cwd: app.config.cwd.clone(),
            tool_name: None,
            tool_input: None,
            tool_result: None,
            prompt: prompt.clone(),
        };
        let result = execute_hook(hook, &context).await;
        if app.config.hook_display != HookDisplay::Hidden {
            app.chat.chat_messages.push(ChatMessage::Hook(crate::tui::messages::HookMessage {
                event,
                hook_name: hook.name.clone(),
                success: result.success(),
                stdout: result.stdout.clone(),
                stderr: result.stderr.clone(),
                tool_name: None,
            }));
            app.chat.auto_scroll = true;
        }
        if result.blocked() {
            blocked = true;
            blocked_error = result.stderr.clone();
            break;
        }
        if result.success() {
            outputs.push((hook.name.clone(), result.stdout.clone()));
        }
    }
    (blocked, blocked_error, outputs)
}

/// Run all matching hooks for `event` on the agent-loop side.
///
/// Sends one `AgentEvent::HookMessage` per executed hook over `tx` (always
/// carrying the real stderr). A non-matching tool skips the hook entirely
/// (no shell spawn, no TUI entry). A blocking hook (exit 2) stops the run
/// for `PreToolUse` (a veto); for `PostToolUse` the run continues (a
/// blocked hook is an objection, not a veto).
///
/// Returns `(blocked, blocked_error, results)` where `results` is the full
/// `HookResult` per executed hook, in registry order; the caller
/// interprets them (`PreToolUse` collects stdout, `PostToolUse` builds the
/// framed instruction).
pub(crate) async fn run_tool_hooks(
    tx: &mpsc::Sender<AgentEvent>,
    registry: &HookRegistry,
    event: HookEvent,
    cwd: &Path,
    tool_name: &str,
    tool_input: &serde_json::Value,
    tool_result: Option<&serde_json::Value>,
) -> (bool, String, Vec<(String, HookResult)>) {
    let stop_on_block = matches!(event, HookEvent::PreToolUse);
    let mut blocked = false;
    let mut blocked_error = String::new();
    let mut results: Vec<(String, HookResult)> = Vec::new();
    for hook in registry.hooks(event) {
        // Tool-name matcher: a non-matching tool skips the hook
        // entirely (no shell spawn, no TUI entry).
        if !hook.matcher.matches(tool_name) {
            continue;
        }
        let context = HookContext {
            event,
            hook_name: hook.name.clone(),
            cwd: cwd.to_path_buf(),
            tool_name: Some(tool_name.to_string()),
            tool_input: Some(tool_input.clone()),
            tool_result: tool_result.cloned(),
            prompt: None,
        };
        let result = execute_hook(hook, &context).await;
        let _ = tx
            .send(AgentEvent::HookMessage {
                event,
                hook_name: hook.name.clone(),
                success: result.success(),
                stdout: result.stdout.clone(),
                stderr: result.stderr.clone(),
                tool_name: Some(tool_name.to_string()),
            })
            .await;
        let is_blocked = result.blocked();
        if is_blocked {
            blocked = true;
            blocked_error = result.stderr.clone();
        }
        results.push((hook.name.clone(), result));
        if is_blocked && stop_on_block {
            break;
        }
    }
    (blocked, blocked_error, results)
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
