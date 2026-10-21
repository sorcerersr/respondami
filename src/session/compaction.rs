//! Session compaction — reduces context window usage.
//!
//! Two strategies share one engine:
//! - [`LlmCompaction`] — model-based summarization (the pre-existing path).
//! - [`VccCompaction`] — algorithmic, LLM-free "vector context checkpoint".
//!
//! The active strategy is chosen by `compaction.mode` in config. The engine
//! owns the shared settings, the trigger check, and the `perform()` lifecycle
//! (compute → apply). Each strategy's `compute()` is spawn-safe: it takes
//! owned/cloned data and returns a [`CompactionPlan`].

pub mod llm;
pub mod vcc;

#[cfg(test)]
mod vcc_tests;

use super::entry::SessionEntry;
use super::manager::SessionStore;
use crate::config::{CompactionMode, Config};
use crate::provider::Provider;
use crate::skills::Skill;
use async_trait::async_trait;
use std::path::{Path, PathBuf};

pub use llm::LlmCompaction;
pub use vcc::VccCompaction;

/// Result of `compute()`. Spawn-safe (all owned data).
#[derive(Debug)]
pub struct CompactionPlan {
    pub summary: String,
    pub cut_index: usize,
    pub tokens_before: u32,
}

/// Shared compaction settings (identical across both strategies).
#[derive(Debug, Clone)]
pub struct CompactionSettings {
    pub enabled: bool,
    pub reserve_tokens: u32,
    pub keep_recent_tokens: u32,
}

impl CompactionSettings {
    /// Minimum number of messages to keep after compaction.
    ///
    /// Prevents over-aggressive compaction where a single large response
    /// could compact the entire conversation history in one pass.
    pub const MIN_KEEP_MESSAGES: usize = 4;

    #[must_use]
    pub fn from_config(compaction: &crate::config::CompactionConfig) -> Self {
        Self {
            enabled: compaction.enabled,
            reserve_tokens: compaction.reserve_tokens,
            keep_recent_tokens: compaction.keep_recent_tokens,
        }
    }

    /// Check if compaction should be triggered.
    #[must_use]
    pub fn should_compact(&self, current_tokens: u32, context_window: u32) -> bool {
        if !self.enabled {
            return false;
        }
        current_tokens >= context_window.saturating_sub(self.reserve_tokens)
    }

    /// Estimate token count from a string (rough: chars / 4).
    #[must_use]
    pub fn estimate_tokens(text: &str) -> u32 {
        (text.len() as u32).saturating_div(4)
    }
}

/// A compaction algorithm.
///
/// Implementations are spawn-safe: `compute` takes owned/cloned data and
/// returns a [`CompactionPlan`] without touching a `SessionStore`. The
/// `keep` parameter is a `keep:N` user request (VCC only; LLM ignores it).
#[async_trait]
pub trait CompactionStrategy: Send + Sync {
    /// Compute a compaction plan.
    ///
    /// # Errors
    ///
    /// Returns an error when there is nothing to compact, no valid cut point
    /// exists, or (for LLM) the provider call fails.
    async fn compute(
        &self,
        provider: Provider,
        config: Config,
        entries: Vec<SessionEntry>,
        cwd: PathBuf,
        skills: Vec<Skill>,
        keep: Option<usize>,
    ) -> anyhow::Result<CompactionPlan>;
}

/// Compaction engine — owns the entire compaction lifecycle.
///
/// Holds shared settings and a boxed strategy. `from_config` selects the
/// strategy from `compaction.mode`.
pub struct CompactionEngine {
    pub settings: CompactionSettings,
    pub context_window: u32,
    strategy: Box<dyn CompactionStrategy>,
}

impl std::fmt::Debug for CompactionEngine {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CompactionEngine")
            .field("settings", &self.settings)
            .field("context_window", &self.context_window)
            .finish()
    }
}

impl CompactionEngine {
    /// Create an engine from config, selecting the strategy by mode.
    #[must_use]
    pub fn from_config(config: &Config) -> Self {
        let strategy: Box<dyn CompactionStrategy> = match config.compaction.mode {
            CompactionMode::Llm => Box::new(LlmCompaction),
            CompactionMode::Vcc => Box::new(VccCompaction),
        };
        Self {
            settings: CompactionSettings::from_config(&config.compaction),
            context_window: config.provider.context_window,
            strategy,
        }
    }

    /// Check if compaction should be triggered based on current token usage.
    #[must_use]
    pub fn should_compact(&self, current_tokens: u32) -> bool {
        self.settings.should_compact(current_tokens, self.context_window)
    }

    /// Estimate total context tokens including system overhead.
    ///
    /// Prefers actual LLM usage data from the last Assistant message.
    /// Falls back to character estimates + `system_overhead_tokens`.
    #[must_use]
    pub fn estimate_context_tokens(&self, session: &SessionStore, system_overhead_tokens: u32) -> u32 {
        session.estimate_context_tokens(system_overhead_tokens)
    }

    /// Compute a compaction plan without touching `SessionStore`.
    ///
    /// Spawn-safe — takes owned/cloned data, returns `CompactionPlan`.
    ///
    /// # Errors
    ///
    /// See [`CompactionStrategy::compute`].
    pub async fn compute_compaction(
        &self,
        provider: Provider,
        config: Config,
        entries: Vec<SessionEntry>,
        cwd: PathBuf,
        skills: Vec<Skill>,
        keep: Option<usize>,
    ) -> anyhow::Result<CompactionPlan> {
        self.strategy
            .compute(provider, config, entries, cwd, skills, keep)
            .await
    }

    /// Perform the full compaction cycle.
    ///
    /// Clones entries from `SessionStore`, delegates to `compute_compaction`,
    /// then applies the returned `CompactionPlan`.
    ///
    /// Returns `(tokens_before, tokens_after, messages_removed)` on success.
    ///
    /// # Errors
    ///
    /// - No active session to compact.
    /// - Strategy errors (see [`CompactionStrategy::compute`]).
    pub async fn perform(
        &self,
        session: &mut SessionStore,
        provider: &Provider,
        config: &Config,
        cwd: &Path,
        skills: &[Skill],
    ) -> anyhow::Result<(u32, u32, u32)> {
        if !session.has_active_session() {
            return Err(anyhow::anyhow!("No active session to compact"));
        }

        let plan = self
            .compute_compaction(
                provider.clone(),
                config.clone(),
                session.entries().to_vec(),
                cwd.to_path_buf(),
                skills.to_vec(),
                None,
            )
            .await?;

        let (tb, ta, mr) = session.apply_compaction(plan.summary, plan.cut_index, plan.tokens_before)?;
        Ok((tb, ta, mr))
    }
}

/// Format active skill instructions for re-injection into a compaction summary.
///
/// Skill instructions enter the context only through conversation history
/// (the `activate_skill` tool result or a `/skillname` user message), so a
/// compaction that cuts those messages would lose them. This section
/// restores them deterministically in the summary, using the same tag shape
/// the model already sees from `activate_skill` tool results.
///
/// Returns an empty string when no skills are active. Skills not found in
/// `skills` (e.g. file deleted) or with unreadable files are skipped.
#[must_use]
pub(crate) fn format_active_skills(active: &[String], skills: &[Skill]) -> String {
    if active.is_empty() {
        return String::new();
    }

    let mut blocks = Vec::new();
    for name in active {
        let Some(skill) = skills.iter().find(|s| &s.name == name) else {
            continue;
        };
        let Ok(content) = std::fs::read_to_string(&skill.file_path) else {
            continue;
        };
        // Strip YAML frontmatter (the body after the second `---`).
        let body = content.splitn(3, "---").nth(2).unwrap_or(&content);
        blocks.push(format!(
            "<skill_instructions name=\"{name}\">\n{}\n</skill_instructions>",
            body.trim()
        ));
    }

    if blocks.is_empty() {
        String::new()
    } else {
        format!("<active-skills>\n{}\n</active-skills>", blocks.join("\n"))
    }
}
