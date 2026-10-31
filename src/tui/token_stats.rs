//! Project token statistics — scan session files and aggregate usage data.
//!
//! Computes total input/output tokens, session count, per-session averages,
//! and input/output ratio across all sessions in `.respondami/sessions/`.

use std::path::Path;

use crate::session::{AgentMessage, SessionEntry, Usage};

/// Aggregated token statistics for a project.
#[derive(Debug, Clone, Default)]
pub struct ProjectTokenStats {
    /// Number of sessions scanned.
    pub session_count: usize,
    /// Total input tokens across all sessions (raw sum of every request's
    /// prompt tokens — the upper bound, no prompt caching considered).
    pub total_input_tokens: u64,
    /// Total output tokens across all sessions.
    pub total_output_tokens: u64,
    /// Cache-aware input estimate: sum of the max `prompt_tokens` per session
    /// (same math as the live status-bar counter — assumes the prompt cache
    /// serves the full prior prefix on every request).
    pub effective_input_tokens: u64,
}

impl ProjectTokenStats {
    /// Total tokens (input + output).
    #[must_use]
    pub fn total_tokens(&self) -> u64 {
        self.total_input_tokens + self.total_output_tokens
    }

    /// Cache-aware total: effective input + output (output is unaffected by
    /// prompt caching).
    #[must_use]
    pub fn effective_total_tokens(&self) -> u64 {
        self.effective_input_tokens + self.total_output_tokens
    }

    /// Average input tokens per session. Returns 0 if no sessions.
    #[must_use]
    pub fn avg_input_per_session(&self) -> u64 {
        if self.session_count == 0 {
            return 0;
        }
        self.total_input_tokens / self.session_count as u64
    }

    /// Average output tokens per session. Returns 0 if no sessions.
    #[must_use]
    pub fn avg_output_per_session(&self) -> u64 {
        if self.session_count == 0 {
            return 0;
        }
        self.total_output_tokens / self.session_count as u64
    }

    /// Input percentage of total tokens. Returns 0 if no tokens.
    #[must_use]
    pub fn input_ratio(&self) -> u64 {
        let total = self.total_tokens();
        if total == 0 {
            return 0;
        }
        (self.total_input_tokens * 100) / total
    }

    /// Output percentage of total tokens. Returns 0 if no tokens.
    #[must_use]
    pub fn output_ratio(&self) -> u64 {
        let total = self.total_tokens();
        if total == 0 {
            return 0;
        }
        (self.total_output_tokens * 100) / total
    }

    /// Format a token count as million tokens (e.g., "1.23M").
    #[must_use]
    pub fn format_millions(tokens: u64) -> String {
        let val = tokens as f64 / 1_000_000.0;
        format!("{val:.2}M")
    }
}

/// Compute project token statistics by scanning all session files.
///
/// Reads every `.jsonl` file in `.respondami/sessions/`, parses `Assistant`
/// messages, and sums their `Usage.prompt_tokens` and `Usage.completion_tokens`.
/// Also tracks the per-session max `prompt_tokens` for the cache-aware
/// `effective_input_tokens` estimate (each session has its own cache prefix,
/// so the max is per session, not global).
///
/// Returns `Ok(default)` when the sessions directory does not exist or contains
/// no valid session files ("no sessions"), and `Err(message)` when a read error
/// occurs (the directory can't be listed, or a file can't be read) so the dialog
/// can distinguish an empty result from a failure.
pub fn compute_project_stats(cwd: &Path) -> Result<ProjectTokenStats, String> {
    let sessions_dir = cwd.join(".respondami").join("sessions");
    if !sessions_dir.exists() {
        return Ok(ProjectTokenStats::default());
    }

    let mut session_count = 0usize;
    let mut total_input: u64 = 0;
    let mut total_output: u64 = 0;
    let mut effective_input: u64 = 0;

    let entries = std::fs::read_dir(&sessions_dir)
        .map_err(|e| format!("cannot list {}: {e}", sessions_dir.display()))?;

    for entry in entries {
        let entry = match entry {
            Ok(e) => e,
            Err(e) => {
                return Err(format!("cannot read entry in {}: {e}", sessions_dir.display()));
            }
        };

        let path = entry.path();
        if path.extension().is_some_and(|ext| ext == "jsonl") {
            session_count += 1;
            let content = match std::fs::read_to_string(&path) {
                Ok(c) => c,
                Err(e) => {
                    return Err(format!("cannot read {}: {e}", path.display()));
                }
            };

            // Per-session max prompt: each session has its own cache prefix,
            // so the effective estimate uses the max within this file, not a
            // global max.
            let mut session_max_prompt: u64 = 0;

            for (i, line) in content.lines().enumerate() {
                let line = line.trim();
                if line.is_empty() {
                    continue;
                }
                match serde_json::from_str::<SessionEntry>(line) {
                    Ok(SessionEntry::Message {
                        message: AgentMessage::Assistant { usage, .. },
                        ..
                    }) => {
                        if let Some(Usage {
                            prompt_tokens,
                            completion_tokens,
                            ..
                        }) = usage.as_ref()
                        {
                            total_input += *prompt_tokens as u64;
                            total_output += *completion_tokens as u64;
                            session_max_prompt = session_max_prompt.max(*prompt_tokens as u64);
                        }
                    }
                    Ok(_) => {}
                    Err(e) => {
                        tracing::warn!(
                            "Skipping corrupted line {} in {}: {}",
                            i + 1,
                            path.display(),
                            e
                        );
                    }
                }
            }

            effective_input += session_max_prompt;
        }
    }

    if session_count == 0 {
        return Ok(ProjectTokenStats::default());
    }

    Ok(ProjectTokenStats {
        session_count,
        total_input_tokens: total_input,
        total_output_tokens: total_output,
        effective_input_tokens: effective_input,
    })
}
