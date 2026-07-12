//! Main application state — thin facade over focused sub-structs.
//!
//! Owns `EditorState`, `ChatState`, `AgentState`, `SessionState`, `ModalState`,
//! `ConfigState`, and `UIState`. Provides message helpers, token usage accumulation,
//! history navigation, and agent event draining.

use std::collections::HashSet;
use std::path::PathBuf;
use std::time::Instant;

use super::agent_event::AgentEvent;
use super::agent_state::AgentState;
use super::chat_state::ChatState;
use super::config_state::ConfigState;
use super::editor_state::EditorState;
use super::messages::{
    AssistantMessage, ChatMessage, CompactionMessage, SystemMessage, ThinkingMessage, ToolCallMessage,
    ToolCallVariant, UserMessage,
};
use super::messages::tool_call::MAX_CONTENT_LINES;
use super::modal_state::ModalState;
use super::session_state::SessionState;
use super::ui_state::UIState;
use crate::config::Config;
use crate::session::{RequestTokenUsage, SessionStore, Usage};
use crate::tools::ToolRegistry;

/// The main application state.
///
/// Thin facade that owns focused sub-structs for each domain:
/// - `EditorState`: input buffer, cursor, autocomplete
/// - `ChatState`: messages, scrolling, viewport
/// - `AgentState`: streaming content, pending tool calls, token tracking
/// - `SessionState`: session persistence, token usage
/// - `ModalState`: app state, popups, command palette
/// - `ConfigState`: config, model, skills, project context
/// - `UIState`: animations, terminal dimensions, status bar
pub struct App {
    pub editor: EditorState,
    pub chat: ChatState,
    pub agent: AgentState,
    pub session: SessionState,
    pub modal: ModalState,
    pub config: ConfigState,
    pub ui: UIState,
    pub tool_registry: ToolRegistry,
    /// Set of skill names that have been activated (by user command or model tool call).
    /// Only activated skills have their hooks fire.
    pub active_skills: HashSet<String>,
    /// In-flight compaction task from manual compaction via command palette.
    /// `Some` = compaction running in background, UI shows sweep animation.
    pub compaction_task: Option<tokio::task::JoinHandle<anyhow::Result<crate::session::CompactionPlan>>>,
}

impl std::fmt::Debug for App {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("App")
            .field("editor", &self.editor)
            .field("chat", &self.chat)
            .field("agent", &self.agent)
            .field("session", &self.session)
            .field("modal", &self.modal)
            .field("config", &self.config)
            .field("ui", &self.ui)
            .field("tool_registry", &self.tool_registry)
            .field("active_skills", &self.active_skills)
            .field("compaction_task", &self.compaction_task.is_some())
            .finish()
    }
}

impl App {
    pub fn new(config: Config, cwd: PathBuf) -> Self {
        let model_name = config.provider.model.clone();
        let context_window = config.provider.context_window;
        let thinking_display = config.ui.thinking_display;
        let tool_output_expanded = config.ui.tool_output_expanded;
        let hook_display = config.ui.hook_display;

        // Load AGENTS.md — capture path for welcome screen, set state for popup
        let (state, agents_md_path) = match crate::agents_md::load_agents_md(&cwd) {
            Ok(Some((_, path))) => (super::mode::AppState::Idle, Some(path)),
            Ok(None) => (super::mode::AppState::InitPopup, None),
            Err(e) => {
                tracing::warn!("Failed to load AGENTS.md: {}", e);
                (super::mode::AppState::InitPopup, None)
            }
        };

        // Resolve RTK state (path + version check)
        let rtk_state = crate::tools::rtk::resolve_rtk_state();
        let rtk_warning = (rtk_state.path.is_some() && !rtk_state.version_ok).then(|| (
                "rtk too old (need ≥ 0.23.0), rewrite disabled".to_string(),
                Instant::now(),
            ));

        // Discover skills (global + project, project wins on collision)
        let (skills, skill_diagnostics) = crate::skills::load_skills(&Config::config_dir(), &cwd);
        for diag in &skill_diagnostics {
            tracing::info!("Skill {}: {} ({})", diag.level, diag.message, diag.path.display());
        }

        // Load hooks from directory-based sources
        let hook_registry = crate::hooks::load_all_hooks(&Config::config_dir(), &cwd);

        Self {
            editor: EditorState::new(),
            chat: ChatState::new(),
            agent: AgentState::new(),
            session: SessionState::new(SessionStore::new(&cwd)),
            modal: ModalState {
                state,
                ..ModalState::new()
            },
            config: ConfigState {
                config,
                model_name,
                context_window,
                thinking_display,
                tool_output_expanded,
                hook_display,
                cwd,
                rtk_state,
                skills,
                agents_md_path,
                hook_registry,
            },
            ui: UIState {
                status_bar_message: rtk_warning,
                ..UIState::new()
            },
            tool_registry: ToolRegistry::new(),
            active_skills: HashSet::new(),
            compaction_task: None,
        }
    }

    /// Initialize tracker with restored session totals.
    pub fn restore_tracker(&mut self, session_tokens: u32, session_seconds: f64) {
        self.agent.tracker = crate::context::TokenRateTracker::restore(session_tokens, session_seconds);
    }

    /// Activate a skill by name. Returns true if the skill was not already active.
    pub fn activate_skill(&mut self, skill_name: &str) -> bool {
        self.active_skills.insert(skill_name.to_string())
    }

    /// Check if the app is in a "working" state (show working indicator).
    #[must_use]
    pub fn is_working(&self) -> bool {
        matches!(
            self.modal.state,
            super::mode::AppState::Streaming | super::mode::AppState::ToolExec | super::mode::AppState::Compacting
        )
    }

    /// Check if the app is in a modal state that blocks normal input.
    #[must_use]
    pub fn is_modal(&self) -> bool {
        matches!(self.modal.state, super::mode::AppState::InitPopup | super::mode::AppState::SessionSelect | super::mode::AppState::CommandPalette | super::mode::AppState::HelpPopup)
    }

    /// Get the current palette filter query, derived from whichever source is active.
    #[must_use]
    pub fn palette_query(&self) -> &str {
        match self.modal.state {
            super::mode::AppState::CommandPalette => &self.modal.command_palette_query,
            _ => "",
        }
    }

    /// Push a token to the streaming content and update display.
    pub fn push_token(&mut self, token: &str) {
        self.agent.streaming_content.push_str(token);
        // Ensure the last message is an Assistant message.
        // After tool execution, the last message is a Tool result,
        // so we need to add a new Assistant message for the follow-up.
        match self.chat.chat_messages.last_mut() {
            Some(ChatMessage::Assistant(msg)) => {
                msg.content.push_str(token);
            }
            _ => {
                // No assistant message at the end — create one
                self.chat.chat_messages.push(ChatMessage::Assistant(AssistantMessage {
                    content: token.to_string(),
                }));
            }
        }
    }

    /// Add a user message to chat.
    pub fn add_user_message(&mut self, content: &str) {
        self.chat.chat_messages.push(ChatMessage::User(UserMessage {
            content: content.to_string(),
        }));
    }

    /// Start streaming — reset state. Assistant message is added lazily by `push_token()`.
    /// Stores the user message for fallback token estimation.
    pub fn start_streaming(&mut self, user_message: &str) {
        self.modal.state = super::mode::AppState::Streaming;
        self.agent.streaming_content.clear();
        self.agent.pending_tool_calls.clear();
        self.agent.pending_tool_call_ids.clear();
        self.agent.current_user_message = Some(user_message.to_string());
    }

    /// Add a pending (executing) tool call message.
    ///
    /// Creates a `Pending` variant showing tool name + args.
    /// Tracks the `tool_call_id` → `chat_messages` index in `pending_tool_call_ids`.
    pub fn add_pending_tool_call(&mut self, tool_call_id: &str, tool_name: &str, tool_args: &serde_json::Value) {
        let idx = self.chat.chat_messages.len();
        self.chat.chat_messages.push(ChatMessage::ToolCall(ToolCallMessage {
            variant: ToolCallVariant::Pending {
                tool_name: tool_name.to_string(),
                tool_args: tool_args.clone(),
                output: String::new(),
                animation_start: Some(Instant::now()),
                effect_added: false,
            },
        }));
        self.agent.pending_tool_call_ids.insert(tool_call_id.to_string(), idx);
    }

    /// Append streaming output to the last pending tool call message.
    ///
    /// Finds the most recent Pending variant and appends text to its output field.
    pub fn append_tool_output(&mut self, text: &str) {
        // Find the last Pending message (tool calls execute sequentially)
        for msg in self.chat.chat_messages.iter_mut().rev() {
            if let ChatMessage::ToolCall(tool_msg) = msg
                && let ToolCallVariant::Pending { output, .. } = &mut tool_msg.variant
            {
                output.push_str(text);
                return;
            }
        }
    }

    /// Update a pending tool call message with execution results.
    ///
    /// Finds the pending message by `tool_call_id` and replaces it with the result variant.
    /// Removes the entry from `pending_tool_call_ids`.
    pub fn update_tool_call_result(
        &mut self,
        tool_call_id: &str,
        result: &str,
        has_error: bool,
        rtk_original: Option<String>,
    ) {
        if let Some(idx) = self.agent.pending_tool_call_ids.remove(tool_call_id) {
            // Extract tool_name and tool_args from the pending message
            let (tool_name, tool_args) = if idx < self.chat.chat_messages.len() {
                match &self.chat.chat_messages[idx] {
                    ChatMessage::ToolCall(msg) => match &msg.variant {
                        ToolCallVariant::Pending { tool_name, tool_args, .. } => {
                            (tool_name.clone(), Some(tool_args.clone()))
                        }
                        _ => ("unknown".to_string(), None),
                    },
                    _ => ("unknown".to_string(), None),
                }
            } else {
                ("unknown".to_string(), None)
            };
            let tool_args_ref = tool_args.as_ref();
            // Replace pending message with result
            self.chat.chat_messages[idx] = ChatMessage::ToolCall(ToolCallMessage {
                variant: self.build_tool_call_variant(&tool_name, result, has_error, tool_args_ref, rtk_original),
            });
        }
    }

    /// Build a `ToolCallVariant` from tool name, result, and args.
    #[must_use]
    pub fn build_tool_call_variant(
        &self,
        tool_name: &str,
        result: &str,
        has_error: bool,
        tool_args: Option<&serde_json::Value>,
        rtk_original: Option<String>,
    ) -> ToolCallVariant {
        super::messages::tool_call::build_tool_call_variant(
            tool_name,
            result,
            has_error,
            tool_args.cloned(),
            rtk_original,
            self.config.tool_output_expanded,
        )
    }

    /// Add a compaction message.
    pub fn add_compaction_message(&mut self, tokens_saved: u32, message_count: u32) {
        self.chat.chat_messages.push(ChatMessage::Compaction(CompactionMessage {
            tokens_saved,
            message_count,
        }));
    }

    /// Add a system message to chat.
    pub fn add_system_message(&mut self, content: &str) {
        self.chat.chat_messages.push(ChatMessage::System(SystemMessage {
            content: content.to_string(),
        }));
    }

    /// Add a thinking block message to chat.
    pub fn add_thinking_message(&mut self) {
        self.chat.chat_messages.push(ChatMessage::Thinking(ThinkingMessage {
            reasoning: String::new(),
        }));
    }

    /// Append reasoning text to the most recent Thinking message.
    pub fn push_reasoning(&mut self, text: &str) {
        if let Some(ChatMessage::Thinking(msg)) = self.chat.chat_messages.iter_mut().rev().find(|m| matches!(m, ChatMessage::Thinking(_))) {
            msg.reasoning.push_str(text);
        }
    }

    /// Accumulate token usage from a provider response.
    ///
    /// Within a single request: takes the max of each field (deduplicates
    /// repeated SSE usage events). Between requests: only adds the delta.
    ///
    /// Also updates session-level token counters for accurate percentage display.
    ///
    /// Mirrors zed's `accumulate_token_usage` pattern.
    pub fn accumulate_token_usage(&mut self, usage: &Usage) {
        let current = RequestTokenUsage {
            input_tokens: self.session.current_request_usage.input_tokens.max(usage.prompt_tokens),
            output_tokens: self.session.current_request_usage.output_tokens.max(usage.completion_tokens),
            estimated: false,
        };

        // Add only the delta to cumulative
        let delta = current.delta(&self.session.current_request_usage);
        self.session.cumulative_usage.input_tokens += delta.input_tokens;
        self.session.cumulative_usage.output_tokens += delta.output_tokens;

        // Update session-level counters for percentage display (actual context window usage)
        self.session.session_prompt_tokens += delta.input_tokens;
        self.session.session_completion_tokens += delta.output_tokens;

        // Clear the estimated flag — we now have real usage data.
        self.session.current_request_usage = current;
    }

    /// Reset current request usage at the start of a new turn.
    pub fn reset_request_usage(&mut self) {
        self.session.current_request_usage = RequestTokenUsage::default();
    }

    /// Clear chat display (preserves session).
    pub fn clear_chat(&mut self) {
        self.chat.chat_messages.clear();
    }

    /// Discover entries for @-autocomplete.
    pub fn discover_files(&mut self) {
        self.editor.discovered_files = super::editor::FileDiscovery::discover_entries(&self.config.cwd);
    }

    /// Toggle expand/collapse state for all tool call output messages.
    /// Flips the global state and applies it to all completed tool calls.
    /// Pending messages are skipped (not expandable).
    pub fn toggle_all_tool_output(&mut self) {
        self.config.tool_output_expanded = !self.config.tool_output_expanded;
        let expanded = self.config.tool_output_expanded;

        use super::messages::tool_call::ToolCallRender;
        for msg in &mut self.chat.chat_messages {
            if let ChatMessage::ToolCall(tool_msg) = msg {
                match &mut tool_msg.variant {
                    ToolCallVariant::Bash(m) => {
                        if m.content_lines().len() > MAX_CONTENT_LINES {
                            m.expanded = expanded;
                        }
                    }
                    ToolCallVariant::Read(m) => {
                        if m.content_lines().len() > MAX_CONTENT_LINES {
                            m.expanded = expanded;
                        }
                    }
                    ToolCallVariant::Write(m) => {
                        if m.content_lines().len() > MAX_CONTENT_LINES {
                            m.expanded = expanded;
                        }
                    }
                    ToolCallVariant::Edit(m) => {
                        if m.content_lines().len() > MAX_CONTENT_LINES {
                            m.expanded = expanded;
                        }
                    }
                    ToolCallVariant::Unknown(m) => {
                        if m.content_lines().len() > MAX_CONTENT_LINES {
                            m.expanded = expanded;
                        }
                    }
                    ToolCallVariant::Pending { .. } => {
                        // Skip pending messages
                    }
                }
            }
        }

        let state = if expanded { "expanded" } else { "collapsed" };
        self.add_system_message(&format!("Tool output: {state}"));
        self.chat.auto_scroll = true;
    }

    /// Fuzzy match entries for autocomplete.
    #[must_use]
    pub fn fuzzy_match_files(&self, query: &str, show_hidden: bool) -> Vec<super::editor::FileMatch> {
        super::editor::FileDiscovery::fuzzy_match(&self.editor.discovered_files, query, 100, show_hidden)
    }

    /// Save current config to disk immediately.
    ///
    /// # Errors
    ///
    /// - File I/O errors when writing the config file.
    pub fn save_config(&self) -> anyhow::Result<()> {
        self.config.config.save()
    }

    /// List sessions for /resume.
    ///
    /// # Errors
    ///
    /// - File I/O errors when reading the sessions directory.
    pub fn list_sessions(&self) -> anyhow::Result<Vec<crate::session::SessionMeta>> {
        self.session.session_store.list_sessions()
    }

    /// Drain remaining agent events from the channel before finalizing a turn.
    ///
    /// `Done` or `NeedsCompaction` may arrive while `Token`/`Reasoning`/`Usage` events
    /// are still buffered in the channel (capacity 256). This method processes all
    /// pending events to prevent silent data loss.
    pub fn drain_pending_events(&mut self, rx: &mut tokio::sync::mpsc::Receiver<AgentEvent>) {
        while let Ok(evt) = rx.try_recv() {
            match evt {
                AgentEvent::Token(t) => {
                    self.agent.tracker.record(t.len());
                    self.push_token(&t);
                }
                AgentEvent::Reasoning(t) => {
                    self.agent.tracker.record(t.len());
                    self.push_reasoning(&t);
                }
                AgentEvent::Usage(u) => {
                    self.accumulate_token_usage(&u);
                    self.agent.tracker.apply_provider_usage(u.completion_tokens);
                }
                AgentEvent::ToolOutput(text) => {
                    self.append_tool_output(&text);
                }
                _ => {}
            }
        }
    }

    /// Get the current state-specific key handler.
    #[must_use]
    pub fn current_handler(&self) -> crate::key_handler::StateHandler {
        use crate::key_handler::StateHandler;
        match self.modal.state {
            super::mode::AppState::Idle => StateHandler::Idle,
            // Streaming/ToolExec/Compacting are mapped to Idle as a safe fallback.
            // During Streaming/ToolExec, process_agent_events() handles input inline
            // and this handler is never reached. During Compacting, cancellation is
            // handled in the main loop (lib.rs) via JoinHandle::abort().
            super::mode::AppState::Streaming
            | super::mode::AppState::ToolExec
            | super::mode::AppState::Compacting => StateHandler::Idle,
            super::mode::AppState::SessionSelect => StateHandler::SessionSelect,
            super::mode::AppState::InitPopup => StateHandler::InitPopup,
            super::mode::AppState::CommandPalette => StateHandler::CommandPalette,
            super::mode::AppState::HelpPopup => StateHandler::HelpPopup,
        }
    }

    /// Get a user message from history by index (reverse chronological order).
    ///
    /// Index 0 = most recent User message, index 1 = second most recent, etc.
    /// Returns `None` if the index is out of bounds.
    #[must_use]
    pub fn get_history_message(&self, index: usize) -> Option<&str> {
        self.chat.chat_messages
            .iter()
            .rev()
            .filter_map(|m| {
                if let ChatMessage::User(msg) = m {
                    Some(&msg.content)
                } else {
                    None
                }
            })
            .nth(index)
            .map(std::string::String::as_str)
    }

    /// Count the number of User messages in the chat.
    #[must_use]
    pub fn history_message_count(&self) -> usize {
        self.chat.chat_messages.iter().filter(|m| matches!(m, ChatMessage::User(_))).count()
    }

    /// Reset history navigation state.
    pub fn reset_history(&mut self) {
        self.editor.history_index = 0;
        self.editor.saved_input = None;
        self.editor.saved_cursor = 0;
    }
}
