//! Session compaction — reduces context window usage via LLM summarization.
//!
//! Finds a cut point in the message history, summarizes older messages with the
//! LLM, and replaces them with a compact summary. Uses token-based and count-based
//! cut point strategies. Produces a `CompactionPlan` that is spawn-safe for async
//! execution.

use super::entry::{AgentMessage, SessionEntry};
use super::manager::SessionStore;
use crate::config::Config;
use crate::provider::{ChatRequest, Message, Provider};
use crate::skills::Skill;
use std::path::{Path, PathBuf};

/// Result of `compute_compaction()`. Spawn-safe (all owned data).
#[derive(Debug)]
pub struct CompactionPlan {
    pub summary: String,
    pub cut_index: usize,
    pub tokens_before: u32,
}

/// Compaction settings.
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
}

/// Compaction engine — owns the entire compaction lifecycle.
///
/// Holds settings and provides `should_compact()` and `perform()` methods
/// so all compaction logic lives in one place.
#[derive(Debug, Clone)]
pub struct CompactionEngine {
    pub settings: CompactionSettings,
    pub context_window: u32,
}

impl CompactionEngine {
    /// Create an engine from config.
    #[must_use]
    pub fn from_config(config: &crate::config::Config) -> Self {
        Self {
            settings: CompactionSettings::from_config(&config.compaction),
            context_window: config.provider.context_window,
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

    /// Compute compaction result without touching `SessionStore`.
    ///
    /// Spawn-safe — takes owned/cloned data, returns `CompactionPlan`.
    /// Does everything `perform()` does except the final `apply_compaction()` call.
    ///
    /// # Errors
    ///
    /// - Provider errors during summarization (network, serialization, context overflow).
    pub async fn compute_compaction(
        &self,
        provider: Provider,
        config: Config,
        entries: Vec<SessionEntry>,
        cwd: PathBuf,
        skills: Vec<Skill>,
    ) -> anyhow::Result<CompactionPlan> {
        // Find compaction boundary (start after last compaction or after session header)
        let boundary_start = entries
            .iter()
            .rposition(|e| matches!(e, SessionEntry::Compaction { .. }))
            .map_or(1, |i| i + 1); // Skip session header
        let boundary_end = entries.len();

        if boundary_start >= boundary_end {
            return Err(anyhow::anyhow!("Nothing to compact"));
        }

        // Find cut point (token-based)
        let cut_index = CompactionSettings::find_cut_point(
            &entries,
            boundary_start,
            boundary_end,
            self.settings.keep_recent_tokens,
        )
        .ok_or_else(|| anyhow::anyhow!("No valid cut point found"))?;

        // Count-based fallback when token estimates are too low
        let cut_index = if cut_index <= boundary_start {
            CompactionSettings::find_cut_point_by_count(&entries, boundary_start, boundary_end)
                .ok_or_else(|| anyhow::anyhow!(
                    "Cannot find valid cut point. Token-based: {cut_index}, count-based: none"
                ))?
        } else {
            cut_index
        };

        if cut_index <= boundary_start {
            return Err(anyhow::anyhow!(
                "Cut point ({cut_index}) would not remove any messages (boundary: {boundary_start})"
            ));
        }

        // Minimum floor: ensure at least MIN_KEEP_MESSAGES are preserved.
        let kept_message_count = entries[cut_index..boundary_end]
            .iter()
            .filter(|e| matches!(e, SessionEntry::Message { .. }))
            .count();
        if kept_message_count < CompactionSettings::MIN_KEEP_MESSAGES {
            return Err(anyhow::anyhow!(
                "Cut point would keep only {} messages (minimum: {}). Not compacting.",
                kept_message_count,
                CompactionSettings::MIN_KEEP_MESSAGES,
            ));
        }

        // Estimate tokens before (includes system overhead)
        let system_overhead = crate::turn::estimate_system_overhead_tokens(&cwd, &skills);
        // Build a fake SessionStore just for token estimation
        let tokens_before = self.estimate_tokens_from_entries(&entries, system_overhead);

        // Serialize entries before cut point
        let conversation_text = CompactionSettings::serialize_for_summary(&entries, cut_index);

        if conversation_text.trim().is_empty() {
            return Err(anyhow::anyhow!(
                "No conversation content to summarize (cut_index: {cut_index}, boundary: {boundary_start})"
            ));
        }

        // Get previous summary for incremental compaction
        let previous_summary = entries.iter().rev().find_map(|e| {
            if let SessionEntry::Compaction { summary, .. } = e {
                Some(summary.clone())
            } else {
                None
            }
        });

        // Extract file operations
        let (read_files, modified_files) =
            CompactionSettings::extract_file_operations(&entries, cut_index);

        // Build summarization prompt
        let max_summary_tokens = std::cmp::min(
            self.settings.reserve_tokens,
            self.context_window.saturating_div(4),
        );
        let user_prompt = CompactionSettings::summarization_user_prompt(
            &conversation_text,
            previous_summary.as_deref(),
            max_summary_tokens,
        );

        // Call LLM for summarization (non-streaming)
        let summary = Self::generate_summary(&provider, &config, &user_prompt).await?;

        if summary.trim().is_empty() {
            return Err(anyhow::anyhow!("LLM returned empty summary for compaction"));
        }

        // Format file operations and append to summary
        let file_ops = CompactionSettings::format_file_operations(&read_files, &modified_files);
        let full_summary = if file_ops.is_empty() {
            summary
        } else {
            format!("{summary}{file_ops}")
        };

        Ok(CompactionPlan {
            summary: full_summary,
            cut_index,
            tokens_before,
        })
    }

    /// Estimate tokens from entries (used by `compute_compaction` which has no `SessionStore`).
    fn estimate_tokens_from_entries(&self, entries: &[SessionEntry], system_overhead: u32) -> u32 {
        // Try to get actual usage from the last Assistant message
        for entry in entries.iter().rev() {
            if let SessionEntry::Message { message: AgentMessage::Assistant { usage, .. }, .. } = entry
                && let Some(u) = usage.as_ref()
            {
                return u.prompt_tokens;
            }
        }

        // Fallback: character estimates + system overhead
        let mut total: u32 = 0;
        for entry in entries {
            if let SessionEntry::Message { message, .. } = entry {
                total += CompactionSettings::estimate_tokens(message.content());
            }
        }
        total.saturating_add(system_overhead)
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
    /// - Provider errors during summarization (network, serialization, context overflow).
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

        let plan = self.compute_compaction(
            provider.clone(),
            config.clone(),
            session.entries().to_vec(),
            cwd.to_path_buf(),
            skills.to_vec(),
        ).await?;

        let (tb, ta, mr) = session.apply_compaction(plan.summary, plan.cut_index, plan.tokens_before)?;
        Ok((tb, ta, mr))
    }

    /// Generate a summary using the provider's non-streaming completion.
    async fn generate_summary(
        provider: &Provider,
        config: &crate::config::Config,
        user_prompt: &str,
    ) -> anyhow::Result<String> {
        let system_prompt = CompactionSettings::summarization_system_prompt();

        let messages = vec![
            Message::System { content: system_prompt },
            Message::User { content: user_prompt.to_string() },
        ];

        let max_tokens = std::cmp::min(
            config.compaction.reserve_tokens,
            config.provider.context_window.saturating_div(4),
        );

        let request = ChatRequest {
            messages,
            model: config.provider.model.clone(),
            tools: Vec::new(),
            stream: false,
            max_tokens: Some(max_tokens),
        };

        let response = provider.complete(&request).await?;
        Ok(response.content)
    }
}

impl CompactionSettings {
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

    /// Find the cut point in entries — returns index of first entry to keep.
    ///
    /// Walks entries in reverse from `end_index`, accumulating token estimates
    /// until `keep_recent_tokens` is reached. Respects compaction boundaries
    /// and only cuts at valid positions (never at tool results).
    ///
    /// Returns `None` if no valid cut point found (keep everything).
    /// Returns `start_index` when total estimated tokens < `keep_recent_tokens`
    /// (caller should use count-based fallback in this case).
    #[must_use]
    pub fn find_cut_point(
        entries: &[SessionEntry],
        start_index: usize,
        end_index: usize,
        keep_recent_tokens: u32,
    ) -> Option<usize> {
        if start_index >= end_index {
            return None;
        }

        let mut accumulated = 0u32;
        let mut last_valid_cut: Option<usize> = None;

        // Walk from newest to oldest
        for i in (start_index..end_index).rev() {
            let entry = &entries[i];

            // Stop at compaction boundaries — never cut through previous compaction
            if matches!(entry, SessionEntry::Compaction { .. }) {
                break;
            }

            let tokens = match entry {
                SessionEntry::Message { message, .. } => {
                    // Tool results must follow their tool calls — not valid cut points
                    if matches!(message, AgentMessage::Tool { .. }) {
                        continue;
                    }
                    Self::estimate_tokens(message.content())
                }
                SessionEntry::ModelChange { .. } => 50,
                SessionEntry::Session { .. }
                | SessionEntry::Custom { .. }
                | SessionEntry::Compaction { .. } => continue,
            };

            accumulated += tokens;
            if accumulated >= keep_recent_tokens {
                // Return i+1 (keep from next entry), not i.
                // Entry i pushed us over the budget — it should be COMPACTED, not kept.
                return Some(std::cmp::min(i + 1, end_index));
            }

            // This is a valid cut point (user, assistant, system, or model change)
            last_valid_cut = Some(i);
        }

        // If we accumulated less than keep_recent_tokens, return the first valid cut
        // (meaning we'd compact everything from start_index to end_index).
        // Caller should validate this produces actual reduction.
        if accumulated > 0 {
            last_valid_cut
        } else {
            None
        }
    }

    /// Count-based fallback: find cut point that keeps roughly half the messages.
    ///
    /// Used when token-based cut point fails (e.g., estimates too low relative to
    /// `keep_recent_tokens`). Walks valid cut points and returns the midpoint.
    ///
    /// Returns `None` if there are too few entries to meaningfully compact.
    #[must_use]
    pub fn find_cut_point_by_count(
        entries: &[SessionEntry],
        start_index: usize,
        end_index: usize,
    ) -> Option<usize> {
        if start_index >= end_index {
            return None;
        }

        let mut valid_cuts: Vec<usize> = Vec::new();

        for (i, entry) in entries[start_index..end_index].iter().enumerate() {
            let idx = start_index + i;
            match entry {
                SessionEntry::Message { message, .. } => {
                    // Tool results must follow their tool calls — not valid cut points
                    if matches!(message, AgentMessage::Tool { .. }) {
                        continue;
                    }
                    valid_cuts.push(idx);
                }
                SessionEntry::ModelChange { .. } => {
                    valid_cuts.push(idx);
                }
                SessionEntry::Session { .. }
                | SessionEntry::Custom { .. }
                | SessionEntry::Compaction { .. } => continue,
            }
        }

        // Need at least 2 valid cut points to compact at least one entry
        if valid_cuts.len() < 2 {
            return None;
        }

        // Keep the second half (newest half), compact the first half (oldest half)
        let midpoint = valid_cuts.len() / 2;
        Some(valid_cuts[midpoint])
    }

    /// Serialize entries before the cut point into a text block for summarization.
    ///
    /// Includes thinking content as `[Assistant thinking]` blocks (pi-coding-agent style).
    #[must_use]
    pub fn serialize_for_summary(entries: &[SessionEntry], cut_index: usize) -> String {
        let mut lines = Vec::new();
        for (i, entry) in entries.iter().enumerate() {
            if i >= cut_index {
                break;
            }
            if let SessionEntry::Message { message, .. } = entry {
                match message {
                    AgentMessage::System { content } => {
                        lines.push(format!("[System] {content}"));
                    }
                    AgentMessage::User { content } => {
                        lines.push(format!("[User] {content}"));
                    }
                    AgentMessage::Assistant { content: blocks, .. } => {
                        for block in blocks {
                            match block {
                                crate::session::ContentBlock::Thinking { thinking } => {
                                    if !thinking.is_empty() {
                                        lines.push(format!("[Assistant thinking] {thinking}"));
                                    }
                                }
                                crate::session::ContentBlock::Text { text } => {
                                    if !text.is_empty() {
                                        lines.push(format!("[Assistant] {text}"));
                                    }
                                }
                                crate::session::ContentBlock::ToolCall { tool_call } => {
                                    lines.push(format!("[Tool] {}: {}", tool_call.name, tool_call.arguments));
                                }
                            }
                        }
                    }
                    AgentMessage::Tool { result, .. } => {
                        let truncated = if result.len() > 4096 {
                            format!("{}... (truncated, {} chars total)", &result[..4096], result.len())
                        } else {
                            result.clone()
                        };
                        lines.push(format!("[Tool Result] {truncated}"));
                    }
                }
            }
        }
        lines.join("\n")
    }

    /// Extract file operations from entries before the cut point.
    /// Returns (`read_files`, `modified_files`) sorted lists.
    #[must_use]
    pub fn extract_file_operations(entries: &[SessionEntry], cut_index: usize) -> (Vec<String>, Vec<String>) {
        use crate::session::ContentBlock;

        let mut read_files = std::collections::HashSet::new();
        let mut modified_files = std::collections::HashSet::new();

        for (i, entry) in entries.iter().enumerate() {
            if i >= cut_index {
                break;
            }
            if let SessionEntry::Message { message, .. } = entry
                && let AgentMessage::Assistant { content: blocks, .. } = message
            {
                for block in blocks {
                    if let ContentBlock::ToolCall { tool_call: tc } = block {
                        let path = tc.arguments.get("path").and_then(|v| v.as_str());
                        match tc.name.as_str() {
                            "read" => {
                                if let Some(p) = path {
                                    read_files.insert(p.to_string());
                                }
                            }
                            "write" | "edit" => {
                                if let Some(p) = path {
                                    modified_files.insert(p.to_string());
                                }
                            }
                            _ => {}
                        }
                    }
                }
            }
        }

        // Files that were both read and modified go in modified only
        let read_only: Vec<String> = read_files
            .into_iter()
            .filter(|f| !modified_files.contains(f))
            .collect();

        let mut read_sorted = read_only;
        read_sorted.sort();
        let mut modified_sorted: Vec<String> = modified_files.into_iter().collect();
        modified_sorted.sort();

        (read_sorted, modified_sorted)
    }

    /// Build the summarization system prompt.
    #[must_use]
    pub fn summarization_system_prompt() -> String {
        "You are a context summarization assistant. Your task is to read a conversation \
         between a user and an AI coding assistant, then produce a structured summary \
         following the exact format specified.\n\n\
         Do NOT continue the conversation. Do NOT respond to any questions in the conversation. \
         ONLY output the structured summary."
            .to_string()
    }

    /// Build the summarization user prompt.
    /// If `previous_summary` is provided, uses update mode.
    #[must_use]
    pub fn summarization_user_prompt(
        conversation_text: &str,
        previous_summary: Option<&str>,
        max_summary_tokens: u32,
    ) -> String {
        let base_prompt = if let Some(prev) = previous_summary {
            format!(
                "<previous-summary>\n{prev}\n</previous-summary>\n\n\
                 The conversation below contains NEW messages to incorporate into the summary above.\n\n\
                 Update the existing structured summary with new information. RULES:\n\
                 - PRESERVE all existing information from the previous summary\n\
                 - ADD new progress, decisions, and context from the new messages\n\
                 - UPDATE the Progress section: move items from \"In Progress\" to \"Done\" when completed\n\
                 - UPDATE \"Next Steps\" based on what was accomplished\n\
                 - PRESERVE exact file paths, function names, and error messages\n\
                 - If something is no longer relevant, you may remove it\n\n"
            )
        } else {
            String::new()
        };

        format!(
            "{base_prompt}<conversation>\n{conversation_text}\n</conversation>\n\n\
             Create a structured context checkpoint summary that another LLM will use to continue the work.\n\n\
             Use this EXACT format:\n\n\
             ## Goal\n\
             [What is the user trying to accomplish?]\n\n\
             ## Constraints & Preferences\n\
             - [Any constraints, preferences, or requirements mentioned by user]\n\n\
             ## Progress\n\
             ### Done\n\
             - [x] [Completed tasks/changes]\n\n\
             ### In Progress\n\
             - [ ] [Current work]\n\n\
             ### Blocked\n\
             - [Issues preventing progress, if any]\n\n\
             ## Key Decisions\n\
             - **[Decision]**: [Brief rationale]\n\n\
             ## Next Steps\n\
             1. [Ordered list of what should happen next]\n\n\
             ## Critical Context\n\
             - [Any data, examples, or references needed to continue]\n\n\
             Keep each section concise. Preserve exact file paths, function names, and error messages.\
             Aim for under {max_summary_tokens} tokens.",
        )
    }

    /// Format file operations as XML tags for appending to summary.
    #[must_use]
    pub fn format_file_operations(read_files: &[String], modified_files: &[String]) -> String {
        let mut sections = Vec::new();
        if !read_files.is_empty() {
            sections.push(format!(
                "<read-files>\n{}\n</read-files>",
                read_files.join("\n")
            ));
        }
        if !modified_files.is_empty() {
            sections.push(format!(
                "<modified-files>\n{}\n</modified-files>",
                modified_files.join("\n")
            ));
        }
        if sections.is_empty() {
            String::new()
        } else {
            format!("\n\n{}", sections.join("\n\n"))
        }
    }
}
