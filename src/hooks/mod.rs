//! Hooks system — shell scripts that execute at lifecycle points.
//!
//! Hooks fire synchronously inside the agent loop, blocking the agent's
//! execution until the hook completes. They can inject context (exit 0),
//! block actions (exit 2), or log non-blocking errors (other exit codes).
//!
//! Two locations for hooks:
//! 1. **Skill frontmatter** — YAML `hooks:` block in `SKILL.md`
//! 2. **Directory-based** — shell scripts in `hooks/<hooktype>/` under
//!    `~/.config/respondami/hooks/` (global) or `.respondami/hooks/` (project)

mod executor;
mod loader;

pub use executor::{execute_hook, HookContext, HookResult};
pub use loader::{
    load_all_hooks, load_directory_hooks, Hook, HookEvent, HookSource, HookRegistry,
};

#[cfg(test)]
mod loader_tests;
#[cfg(test)]
mod executor_tests;
