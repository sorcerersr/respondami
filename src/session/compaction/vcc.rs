//! VCC — Vector Context Checkpoint (algorithmic, LLM-free compaction).
//!
//! `VccCompaction` implements [`CompactionStrategy`]. It is deterministic and
//! instant: it picks a user-turn-anchored cut, builds five structured sections
//! from the compacted prefix, renders a ranked brief transcript under a token
//! budget, merges with any previous summary, and re-injects active skills. No
//! model call, no token cost.
//!
//! The submodules hold the ported pi-vcc algorithm:
//! - [`normalize`] — entries → blocks.
//! - [`sections`] — the five section extractors.
//! - [`brief`] — brief transcript rendering.
//! - [`rank`] — block scoring + selection.
//! - [`bash`] — command compression.
//! - [`merge`] — merge with a previous summary.
//! - [`cut`] — cut selection.

pub mod bash;
pub mod brief;
pub mod cut;
pub mod merge;
pub mod normalize;
pub mod rank;
pub mod sections;

#[cfg(test)]
mod bash_tests;
#[cfg(test)]
mod brief_tests;
#[cfg(test)]
mod cut_tests;
#[cfg(test)]
mod merge_tests;
#[cfg(test)]
mod normalize_tests;
#[cfg(test)]
mod rank_tests;
#[cfg(test)]
mod sections_tests;

use super::{CompactionPlan, CompactionStrategy, format_active_skills};
use crate::config::Config;
use crate::provider::Provider;
use crate::session::SessionEntry;
use crate::skills::Skill;
use async_trait::async_trait;
use std::path::PathBuf;

/// Maximum rendered brief lines before capping.
pub const BRIEF_MAX_LINES: usize = 120;
/// Maximum characters per rendered line before wrapping.
const TUI_SAFE_LINE_CHARS: usize = 120;
/// Size-relative brief budget ceiling (tokens).
const RANKED_BRIEF_CEILING_TOKENS: u32 = 2000;
/// Size-relative brief budget slope (tokens per normalized block).
const RANKED_BRIEF_TOKENS_PER_BLOCK: u32 = 15;

/// Algorithmic, LLM-free compaction strategy.
#[derive(Debug, Clone, Copy, Default)]
pub struct VccCompaction;

/// The five summary sections.
#[derive(Debug, Default)]
struct SectionData {
    session_goal: Vec<String>,
    files_and_changes: Vec<String>,
    commits: Vec<String>,
    outstanding_context: Vec<String>,
    user_preferences: Vec<String>,
    brief_transcript: String,
}

fn section(title: &str, items: &[String]) -> String {
    if items.is_empty() {
        return String::new();
    }
    let body = items.iter().map(|i| format!("- {i}")).collect::<Vec<_>>().join("\n");
    format!("[{title}]\n{body}")
}

/// Wrap lines longer than `max_chars` at a word boundary.
fn wrap_long_lines(text: &str, max_chars: usize) -> String {
    let mut out = Vec::new();
    for line in text.lines() {
        if line.len() <= max_chars {
            out.push(line.to_string());
            continue;
        }
        let mut remaining = line.to_string();
        let mut prefix = String::new();
        let mut wrapped = Vec::new();
        while prefix.len() + remaining.len() > max_chars {
            let available = max_chars.saturating_sub(prefix.len()).max(20);
            let split_at = remaining[..available]
                .rfind(' ')
                .filter(|&i| i > available / 2)
                .unwrap_or(available);
            wrapped.push(format!("{}{}", prefix, remaining[..split_at].trim_end()));
            remaining = remaining[split_at..].trim_start().to_string();
            prefix = "    ".to_string();
        }
        if !remaining.is_empty() {
            wrapped.push(format!("{prefix}{remaining}"));
        }
        out.extend(wrapped);
    }
    out.join("\n")
}

/// Format the five sections into a summary string.
fn format_summary(data: &SectionData, cap_brief: bool) -> String {
    let header_parts = [
        section("Session Goal", &data.session_goal),
        section("Files And Changes", &data.files_and_changes),
        section("Commits", &data.commits),
        section("Outstanding Context", &data.outstanding_context),
        section("User Preferences", &data.user_preferences),
    ]
    .into_iter()
    .filter(|s| !s.is_empty())
    .collect::<Vec<_>>();

    let mut parts: Vec<String> = Vec::new();
    if !header_parts.is_empty() {
        parts.push(header_parts.join("\n\n"));
    }
    if !data.brief_transcript.is_empty() {
        let brief = if cap_brief {
            brief::cap_brief_default(&data.brief_transcript)
        } else {
            data.brief_transcript.clone()
        };
        parts.push(brief);
    }
    if parts.is_empty() {
        return String::new();
    }
    wrap_long_lines(&parts.join("\n\n---\n\n"), TUI_SAFE_LINE_CHARS)
}

#[async_trait]
impl CompactionStrategy for VccCompaction {
    async fn compute(
        &self,
        _provider: Provider,
        config: Config,
        entries: Vec<SessionEntry>,
        _cwd: PathBuf,
        skills: Vec<Skill>,
        keep: Option<usize>,
    ) -> anyhow::Result<CompactionPlan> {
        let vcc_config = &config.compaction.vcc;
        let est = crate::agent::token_estimation::TokenEstimator::from_entries(&entries);

        let live_start = cut::find_live_start(&entries);
        let live_len = entries.len() - live_start;
        if live_len <= 2 {
            return Err(anyhow::anyhow!("Too few messages to compact"));
        }

        // Resolve keep.
        let explicit = keep.is_some();
        let keep = if explicit {
            keep.unwrap_or(1)
        } else {
            cut::resolve_smart_keep(
                &entries,
                live_start,
                &est,
                vcc_config.smart_keep_tail,
                false,
                None,
            )
        };

        // Build the cut, then apply the tail budget (default path only).
        let mut cut_result = cut::build_own_cut(&entries, live_start, keep);
        if !explicit {
            cut_result = cut::apply_tail_budget(
                &entries,
                live_start,
                cut_result,
                &est,
                config.compaction.keep_recent_tokens,
            );
        }

        let cut_index = match cut_result {
            cut::CutResult::TooFew => {
                return Err(anyhow::anyhow!("Too few messages to compact"));
            }
            cut::CutResult::CompactAll => entries.len(),
            cut::CutResult::Cut(i) => i,
        };

        // The compacted prefix is [live_start, cut_index).
        let prefix = &entries[live_start..cut_index];
        if prefix.is_empty() {
            return Err(anyhow::anyhow!("Nothing to compact"));
        }

        // Build blocks and sections.
        let blocks = normalize::normalize_entries(prefix);
        if blocks.is_empty() {
            return Err(anyhow::anyhow!("No conversation content to summarize"));
        }

        let goals = sections::extract_goals(&blocks);
        let files = sections::format_file_activity(&blocks);
        let commits = sections::format_commits(&sections::extract_commits(&blocks), 8);
        let outstanding = sections::extract_outstanding_context(&blocks);
        let prefs = sections::dedup_preferences_against_goals(
            sections::extract_preferences(&blocks),
            &goals,
        );

        // Ranked brief under a size-relative token budget: scales with block
        // count between the configured floor and a fixed ceiling.
        let budget_tokens = vcc_config.brief_budget_tokens;
        let block_count = blocks.len() as u32;
        let floor = budget_tokens as usize;
        let ceiling = RANKED_BRIEF_CEILING_TOKENS as usize;
        let slope = RANKED_BRIEF_TOKENS_PER_BLOCK as usize;
        let char_budget = std::cmp::min(
            ceiling,
            std::cmp::max(floor, slope * block_count as usize),
        ) as u32
            * est.chars_per_token() as u32;
        let brief_blocks = rank::select_ranked_brief_blocks(&blocks, Some(char_budget as usize));
        let brief_transcript = brief::compile_brief(&brief_blocks);

        let data = SectionData {
            session_goal: goals,
            files_and_changes: files,
            commits,
            outstanding_context: outstanding,
            user_preferences: prefs,
            brief_transcript,
        };

        let fresh = format_summary(&data, false);
        if fresh.is_empty() {
            return Err(anyhow::anyhow!("No conversation content to summarize"));
        }

        // Merge with a previous summary.
        let previous_summary = entries.iter().rev().find_map(|e| {
            if let SessionEntry::Compaction { summary, .. } = e {
                Some(summary.clone())
            } else {
                None
            }
        });
        let merged = match previous_summary {
            Some(prev) => merge::merge_previous(&prev, &fresh, BRIEF_MAX_LINES),
            None => fresh,
        };

        // Re-inject active skill instructions (deterministic).
        let active_skills = entries
            .first()
            .and_then(|e| match e {
                SessionEntry::Session { activated_skills, .. } => Some(activated_skills.clone()),
                _ => None,
            })
            .unwrap_or_default();
        let skills_section = format_active_skills(&active_skills, &skills);
        let full_summary = if skills_section.is_empty() {
            merged
        } else {
            format!("{merged}\n\n{skills_section}")
        };

        // Tokens before (full context, calibrated).
        let tokens_before =
            crate::agent::token_estimation::estimate_tokens_from_messages(
                &crate::session::build_context_from_entries(&entries),
                &est,
            );

        Ok(CompactionPlan {
            summary: full_summary,
            cut_index,
            tokens_before,
        })
    }
}
