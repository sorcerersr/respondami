//! Session persistence and management.
//!
//! Handles JSONL I/O for session files, context building, compaction application,
//! and token usage tracking.
//!
//! Rust guideline compliant 2026-02-21

use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use anyhow::Context;

use super::compaction::CompactionSettings;
use super::entry::{AgentMessage, SessionEntry, TokenRateEntry};

/// Metadata about a session for listing purposes.
#[derive(Debug, Clone)]
pub struct SessionMeta {
    pub id: String,
    pub timestamp: String,
    pub cwd: String,
    pub model: String,
    pub context_window: u32,
    pub first_message: String,
    pub message_count: u32,
    pub path: PathBuf,
}

/// Stores session data and handles JSONL I/O.
#[derive(Debug)]
pub struct SessionStore {
    sessions_dir: PathBuf,
    current_session_path: Option<PathBuf>,
    current_session_id: Option<String>,
    entries: Vec<SessionEntry>,
}

impl SessionStore {
    /// Create a new session manager for a project directory.
    #[must_use]
    pub fn new(project_dir: &Path) -> Self {
        let sessions_dir = project_dir.join(".respondami").join("sessions");
        fs::create_dir_all(&sessions_dir).ok();
        Self {
            sessions_dir,
            current_session_path: None,
            current_session_id: None,
            entries: Vec::new(),
        }
    }

    /// Get the sessions directory.
    #[must_use]
    pub fn sessions_dir(&self) -> &Path {
        &self.sessions_dir
    }

    /// Check if there's an active session.
    #[must_use]
    pub fn has_active_session(&self) -> bool {
        self.current_session_path.is_some()
    }

    /// Get the current session ID.
    #[must_use]
    pub fn session_id(&self) -> Option<&str> {
        self.current_session_id.as_deref()
    }

    /// Create a new session and return its ID.
    ///
    /// # Panics
    ///
    /// - If writing the session header to disk fails (should not happen in normal operation).
    pub fn create_session(&mut self, model: String, context_window: u32, cwd: String) -> String {
        let id = uuid::Uuid::new_v4().to_string();
        let filename = format!("{id}.jsonl");
        let path = self.sessions_dir.join(&filename);

        let header = SessionEntry::new_session(
            id.clone(),
            chrono::Utc::now().to_rfc3339(),
            cwd,
            model,
            context_window,
        );

        self.entries.clear();
        self.entries.push(header.clone());
        self.append_entry_to_disk(&header, &path).expect("Failed to write session header");

        self.current_session_path = Some(path);
        self.current_session_id = Some(id.clone());
        id
    }

    /// Add a skill to the activated skills list and persist it.
    #[expect(clippy::collapsible_if, reason = "nested guard for activated_skills list check")]
    pub fn add_activated_skill(&mut self, skill_name: &str) {
        if let Some(SessionEntry::Session { activated_skills, .. }) = self.entries.first_mut() {
            if !activated_skills.contains(&skill_name.to_string()) {
                activated_skills.push(skill_name.to_string());
                // Rewrite the session file to disk
                if let Some(ref path) = self.current_session_path {
                    if let Err(e) = self.rewrite_session_file(path) {
                        tracing::error!("Failed to rewrite session file after skill activation: {e}");
                    }
                }
            }
        }
    }

    /// Get the list of activated skills from the current session.
    #[must_use]
    pub fn get_activated_skills(&self) -> Vec<String> {
        if let Some(SessionEntry::Session { activated_skills, .. }) = self.entries.first() {
            activated_skills.clone()
        } else {
            Vec::new()
        }
    }

    /// Set the activated skills list (used during session resume).
    pub fn set_activated_skills(&mut self, skills: Vec<String>) {
        if let Some(SessionEntry::Session { activated_skills, .. }) = self.entries.first_mut() {
            *activated_skills = skills;
        }
    }

    /// Resume an existing session by path.
    ///
    /// # Errors
    ///
    /// - File I/O errors when reading the session file.
    /// - JSON parse errors if individual lines are malformed (skipped with warning).
    pub fn load_session(&mut self, path: &Path) -> anyhow::Result<Vec<SessionEntry>> {
        let content = fs::read_to_string(path)
            .with_context(|| format!("Failed to read session {}", path.display()))?;

        self.entries.clear();
        for (i, line) in content.lines().enumerate() {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            match serde_json::from_str::<SessionEntry>(line) {
                Ok(entry) => self.entries.push(entry),
                Err(e) => {
                    tracing::warn!("Skipping corrupted line {} in {}: {}", i + 1, path.display(), e);
                }
            }
        }

        self.current_session_path = Some(path.to_path_buf());
        if let Some(SessionEntry::Session { id, .. }) = self.entries.first() {
            self.current_session_id = Some(id.clone());
        }

        Ok(self.entries.clone())
    }

    /// Append a message entry to the session.
    ///
    /// # Errors
    ///
    /// - File I/O errors when appending to the session file on disk.
    pub fn append_message(&mut self, parent_id: Option<String>, message: AgentMessage) -> anyhow::Result<String> {
        let entry = SessionEntry::new_message(parent_id, message);
        let id = entry.id();
        self.entries.push(entry.clone());
        if let Some(ref path) = self.current_session_path {
            self.append_entry_to_disk(&entry, path)?;
        }
        Ok(id)
    }

    /// Append any entry to the session.
    ///
    /// # Errors
    ///
    /// - File I/O errors when appending to the session file on disk.
    pub fn append_entry(&mut self, entry: SessionEntry) -> anyhow::Result<String> {
        let id = entry.id();
        self.entries.push(entry.clone());
        if let Some(ref path) = self.current_session_path {
            self.append_entry_to_disk(&entry, path)?;
        }
        Ok(id)
    }

    /// Get all entries.
    #[must_use]
    pub fn entries(&self) -> &[SessionEntry] {
        &self.entries
    }

    /// Build context messages by walking entries.
    /// Injects compaction summaries and skips entries before the cut point.
    #[must_use]
    pub fn build_context(&self) -> Vec<AgentMessage> {
        let mut messages = Vec::new();
        let mut skip_until: Option<String> = None;

        for entry in &self.entries {
            match entry {
                SessionEntry::Session { .. } => continue,
                SessionEntry::Message { message, .. } => {
                    if let Some(ref skip_id) = skip_until {
                        if entry.id() == *skip_id {
                            skip_until = None;
                        } else {
                            continue;
                        }
                    }
                    // Skip system messages from session history — the system
                    // prompt is injected by build_context_with_system() to
                    // avoid duplicate system messages that break templates
                    // requiring system at position 0.
                    if matches!(message, AgentMessage::System { .. }) {
                        continue;
                    }
                    messages.push(message.clone());
                }
                SessionEntry::Compaction {
                    summary,
                    first_kept_entry_id,
                    ..
                } => {
                    // Inject summary as user message (not system) to avoid
                    // breaking Jinja templates that require system messages
                    // at the beginning only.
                    messages.push(AgentMessage::user(format!(
                        "[Previous conversation summary]\n{summary}"
                    )));
                    skip_until = Some(first_kept_entry_id.clone());
                }
                SessionEntry::ModelChange { .. } => {
                    // Model changes are tracked but don't add messages
                }
                SessionEntry::Custom { .. } => {
                    // Custom entries (token-rate, etc.) are metadata only
                }
            }
        }

        messages
    }

    /// Get the last entry ID (for parent chaining).
    #[must_use]
    pub fn last_entry_id(&self) -> Option<String> {
        self.entries.last().map(super::entry::SessionEntry::id)
    }

    /// Get the model name from the session header.
    #[must_use]
    pub fn model_name(&self) -> Option<String> {
        self.entries.iter().find_map(|e| {
            if let SessionEntry::Session { model, .. } = e {
                Some(model.clone())
            } else {
                None
            }
        })
    }

    /// Estimate current context token count from session entries.
    /// Uses character-based estimation (chars / 4) on `build_context()` output.
    ///
    /// This does NOT include system prompt, AGENTS.md, or skills overhead.
    /// Use `estimate_context_tokens()` for a more accurate estimate that includes
    /// system overhead and actual LLM usage data when available.
    #[must_use]
    pub fn estimate_token_count(&self) -> u32 {
        self.build_context()
            .iter()
            .map(|m| CompactionSettings::estimate_tokens(m.content()))
            .sum()
    }

    /// Estimate total context tokens including system overhead.
    ///
    /// Prefers actual LLM usage data from the last Assistant message (most accurate).
    /// Falls back to character estimates + `system_overhead_tokens`.
    ///
    /// `system_overhead_tokens` should include system prompt, AGENTS.md, skills, etc.
    #[must_use]
    pub fn estimate_context_tokens(&self, system_overhead_tokens: u32) -> u32 {
        // Try to get actual usage from the last Assistant message
        for entry in self.entries.iter().rev() {
            if let SessionEntry::Message { message: AgentMessage::Assistant { usage, .. }, .. } = entry
                && let Some(u) = usage.as_ref()
            {
                // prompt_tokens from the LLM = actual context size
                return u.prompt_tokens;
            }
        }

        // Fallback: character estimates + system overhead
        let entry_tokens: u32 = self
            .build_context()
            .iter()
            .map(|m| CompactionSettings::estimate_tokens(m.content()))
            .sum();
        entry_tokens.saturating_add(system_overhead_tokens)
    }

    /// Sum token usage from all saved Assistant messages in the session.
    /// Returns `(total_input_tokens, total_output_tokens)` for tracker restoration.
    #[must_use]
    pub fn total_usage(&self) -> (u32, u32) {
        let mut input: u32 = 0;
        let mut output: u32 = 0;
        for entry in &self.entries {
            if let SessionEntry::Message { message, .. } = entry
                && let AgentMessage::Assistant { usage, .. } = message
                && let Some(u) = usage.as_ref()
            {
                input += u.prompt_tokens;
                output += u.completion_tokens;
            }
        }
        (input, output)
    }

    /// List all sessions in the sessions directory, sorted by mtime (newest first).
    ///
    /// # Errors
    ///
    /// - File I/O errors when reading the sessions directory.
    pub fn list_sessions(&self) -> anyhow::Result<Vec<SessionMeta>> {
        let mut sessions = Vec::new();

        if !self.sessions_dir.exists() {
            return Ok(sessions);
        }

        let mut entries: Vec<fs::DirEntry> = fs::read_dir(&self.sessions_dir)
            .context("Failed to read sessions dir")?
            .filter_map(std::result::Result::ok)
            .filter(|e| e.path().extension().is_some_and(|ext| ext == "jsonl"))
            .collect();

        // Sort by mtime descending
        entries.sort_by_key(|e| {
            e.metadata()
                .ok()
                .and_then(|m| m.modified().ok())
                .map(|t| t.duration_since(std::time::UNIX_EPOCH).unwrap_or_default())
                .unwrap_or_default()
        });
        entries.reverse();

        for dir_entry in entries {
            let path = dir_entry.path();
            let content = match fs::read_to_string(&path) {
                Ok(c) => c,
                Err(_) => continue,
            };

            let mut lines = content.lines().filter(|l| !l.trim().is_empty());

            // Parse header
            let header_line = match lines.next() {
                Some(l) => l,
                None => continue,
            };
            let header: SessionEntry = match serde_json::from_str(header_line) {
                Ok(SessionEntry::Session { version, id, timestamp, cwd, model, context_window, activated_skills }) => {
                    SessionEntry::Session { version, id, timestamp, cwd, model, context_window, activated_skills }
                }
                _ => continue,
            };

            // Find first message and count messages
            let mut first_message = String::new();
            let mut message_count = 0u32;
            for line in lines {
                match serde_json::from_str::<SessionEntry>(line.trim()) {
                    Ok(SessionEntry::Message { message, .. }) => {
                        message_count += 1;
                        if first_message.is_empty()
                            && let AgentMessage::User { content } = &message
                        {
                            first_message = content.chars().take(50).collect();
                        }
                    }
                    Ok(_) => {}
                    Err(_) => {}
                }
            }

            sessions.push(SessionMeta {
                id: match &header {
                    SessionEntry::Session { id, .. } => id.clone(),
                    _ => continue,
                },
                timestamp: match &header {
                    SessionEntry::Session { timestamp, .. } => timestamp.clone(),
                    _ => continue,
                },
                cwd: match &header {
                    SessionEntry::Session { cwd, .. } => cwd.clone(),
                    _ => continue,
                },
                model: match &header {
                    SessionEntry::Session { model, .. } => model.clone(),
                    _ => continue,
                },
                context_window: match &header {
                    SessionEntry::Session { context_window, .. } => *context_window,
                    _ => continue,
                },
                first_message,
                message_count,
                path,
            });
        }

        Ok(sessions)
    }

    /// Append a serialized entry to the session file with fsync.
    fn append_entry_to_disk(&self, entry: &SessionEntry, path: &Path) -> io::Result<()> {
        let json = serde_json::to_string(entry)?;
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)?;
        file.write_all(json.as_bytes())?;
        file.write_all(b"\n")?;
        file.sync_all()?;
        Ok(())
    }

    /// Save a token rate snapshot to the current session.
    pub fn save_token_rate(&mut self, entry: TokenRateEntry) {
        let path = match &self.current_session_path {
            Some(p) => p,
            None => return,
        };
        let data = serde_json::to_value(&entry).unwrap_or_default();
        let custom_entry = SessionEntry::new_custom("token-rate".into(), data);
        self.entries.push(custom_entry.clone());
        self.append_entry_to_disk(&custom_entry, path).ok();
    }

    /// Sum all token-rate entries in the current session.
    /// Returns (`total_tokens`, `total_seconds`) for tracker restoration.
    #[must_use]
    pub fn total_token_rate(&self) -> (u32, f64) {
        let mut total_tokens: u32 = 0;
        let mut total_seconds: f64 = 0.0;
        for entry in &self.entries {
            if let SessionEntry::Custom { custom_type, data } = entry
                && custom_type == "token-rate"
                && let Ok(rate) = serde_json::from_value::<TokenRateEntry>(data.clone())
            {
                total_tokens += rate.tokens;
                total_seconds += rate.seconds;
            }
        }
        (total_tokens, total_seconds)
    }

    /// Find the index of the last compaction entry (if any).
    #[must_use]
    pub fn last_compaction_index(&self) -> Option<usize> {
        self.entries
            .iter()
            .rposition(|e| matches!(e, SessionEntry::Compaction { .. }))
    }

    /// Get the summary from the last compaction entry (if any).
    #[must_use]
    pub fn last_compaction_summary(&self) -> Option<String> {
        self.entries.iter().rev().find_map(|e| {
            if let SessionEntry::Compaction { summary, .. } = e {
                Some(summary.clone())
            } else {
                None
            }
        })
    }

    /// Save a compaction entry and rebuild in-memory state.
    /// Returns (`tokens_before`, `tokens_after`, `messages_removed`).
    ///
    /// # Errors
    ///
    /// - I/O errors when rewriting the session file to disk.
    pub fn apply_compaction(
        &mut self,
        summary: String,
        cut_index: usize,
        tokens_before: u32,
    ) -> anyhow::Result<(u32, u32, u32)> {
        let messages_before = self
            .entries
            .iter()
            .filter(|e| matches!(e, SessionEntry::Message { .. }))
            .count();

        // Find the ID of the first entry to keep
        let first_kept_id = self.entries.get(cut_index).map(super::entry::SessionEntry::id);
        let first_kept_id = match first_kept_id {
            Some(id) => id,
            None => return Ok((tokens_before, tokens_before, 0)),
        };

        // Build compaction entry
        let parent_id = self.last_entry_id();
        let compaction_entry =
            SessionEntry::new_compaction(parent_id, summary.clone(), first_kept_id.clone(), tokens_before);

        // Build new entries: header + compaction + kept entries
        let mut new_entries = Vec::new();

        // Keep session header
        if let Some(header) = self.entries.first().cloned() {
            new_entries.push(header);
        }

        // Add compaction entry
        new_entries.push(compaction_entry);

        // Keep entries from cut_index onward.
        // Clear usage data from Assistant messages to prevent stale prompt_tokens
        // from causing double compaction (estimate_context_tokens prefers usage data).
        for entry in self.entries.iter().skip(cut_index) {
            let mut entry = entry.clone();
            if let SessionEntry::Message { message: AgentMessage::Assistant { usage, .. }, .. } = &mut entry {
                *usage = None;
            }
            new_entries.push(entry);
        }

        let messages_after = new_entries
            .iter()
            .filter(|e| matches!(e, SessionEntry::Message { .. }))
            .count();

        // Rewrite session file
        self.entries = new_entries;
        if let Some(ref path) = self.current_session_path {
            self.rewrite_session_file(path)?;
        }

        let messages_removed = (messages_before.saturating_sub(messages_after)) as u32;
        Ok((tokens_before, self.estimate_token_count(), messages_removed))
    }

    /// Rewrite the entire session file from memory.
    ///
    /// # Errors
    ///
    /// - I/O errors when creating, writing, or flushing the session file.
    fn rewrite_session_file(&self, path: &std::path::Path) -> io::Result<()> {
        let mut content = String::new();
        for entry in &self.entries {
            if let Ok(json) = serde_json::to_string(entry) {
                content.push_str(&json);
                content.push('\n');
            }
        }
        let file = std::fs::File::create(path)?;
        let mut writer = std::io::BufWriter::new(file);
        writer.write_all(content.as_bytes())?;
        writer.flush()?;
        Ok(())
    }
}
