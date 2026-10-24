//! Turn orchestration — coordinates the full lifecycle of a user turn:
//! parse → add message → session → spawn agent → process events.

use std::collections::HashSet;

use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;

use crate::agent::token_estimation::TokenEstimator;
use crate::session::CompactionSettings;
use crate::tui::editor::fuzzy_match_case_insensitive;
use crate::tui::{AgentEvent, App, AppState};

use super::agent_events::process_agent_events;
use super::commands::execute_palette_command;

/// Estimate system overhead tokens (system prompt + AGENTS.md + skills).
/// Used by the pre-prompt compaction check to get an accurate context
/// estimate that includes the full system context.
#[must_use]
pub fn estimate_system_overhead_tokens(
    cwd: &std::path::Path,
    skills: &[crate::skills::Skill],
    est: &TokenEstimator,
) -> u32 {
    let mut overhead: u32 = 0;

    // System prompt
    let system_prompt = crate::agent::get_system_prompt();
    overhead += est.estimate_tokens(system_prompt);

    // AGENTS.md
    if let Ok(Some((content, _))) = crate::agents_md::load_agents_md(cwd) {
        overhead += est.estimate_tokens(&content);
    }

    // Skills XML block
    let skills_xml = crate::skills::format_skills_for_prompt(skills);
    overhead += est.estimate_tokens(&skills_xml);

    overhead
}

/// Snapshot of a turn deferred to background compaction.
///
/// Captured by `run_turn_with_input` when the pre-prompt compaction check
/// fires. The main loop uses `input` to launch the agent when compaction
/// finishes; `rollback_pending_turn` restores pre-turn state on cancel.
#[derive(Debug)]
pub struct PendingTurn {
    /// Input to launch the agent with (may be a skill-injected prompt).
    pub input: String,
    /// Text to restore to the editor on cancel (what the user typed).
    pub restore_input: String,
    /// `true` if the session was auto-created for this turn.
    pub created_session: bool,
    /// Chat message count before this turn (rollback truncates to this).
    pub pre_turn_chat_len: usize,
    /// Active skills before this turn (rollback restores this set).
    pub pre_turn_skills: HashSet<String>,
}

/// Start a turn from the current input buffer.
/// Called when the user presses Enter in idle state.
///
/// # Errors
///
/// - Hook execution errors (exit code != 0).
/// - Agent streaming errors (network, serialization, context overflow).
/// - Session persistence errors (file I/O).
pub async fn start_turn(
    app: &mut App,
    terminal: &mut Terminal<CrosstermBackend<std::io::Stdout>>,
) -> anyhow::Result<bool> {
    // In-progress guard: compaction is running — keep the typed text in the
    // editor (buffer untouched) so a single Enter re-sends once it ends.
    if app.compaction_task.is_some() {
        app.notify_compaction_in_progress();
        return Ok(false);
    }

    let input = app.editor.input_buffer.clone();
    app.editor.input_buffer.clear();
    app.editor.cursor_pos = 0;

    if input.trim().is_empty() {
        return Ok(false);
    }

    // Check for slash commands (palette commands triggered via /).
    // Runs before skill activation: a skill whose name fuzzy-matches a
    // command id is shadowed by the command.
    if let Some((cmd, _args)) = parse_slash_command(&input) {
        return execute_palette_command(app, cmd, terminal).await;
    }

    // Check for /skillname activation
    if let Some(result) = handle_skill_activation(app, &input, terminal).await {
        return Ok(result);
    }

    let pre_turn_skills = app.active_skills.clone();
    run_turn_with_input(app, input, None, pre_turn_skills, terminal).await
}

/// Parse a slash command from input. Returns (`command_id`, args) if matched.
/// Uses fuzzy matching against palette commands.
fn parse_slash_command(input: &str) -> Option<(&'static str, String)> {
    let trimmed = input.trim();
    let rest = trimmed.strip_prefix('/')?;
    let (name, args) = if let Some(space_pos) = rest.find(' ') {
        (&rest[..space_pos], rest[space_pos + 1..].trim().to_string())
    } else {
        (rest.trim(), String::new())
    };
    if name.is_empty() {
        return None;
    }

    let commands = crate::tui::editor::get_palette_commands();

    // Exact match first
    if let Some(cmd) = commands.iter().find(|c| c.id == name) {
        return Some((cmd.id, args));
    }

    // Fuzzy match
    let mut best: Option<(usize, &'static str)> = None;
    for cmd in &commands {
        if let Some(score) = fuzzy_match_case_insensitive(name, cmd.id) {
            match best {
                Some((best_score, _)) if score <= best_score => {}
                _ => {
                    best = Some((score, cmd.id));
                }
            }
        }
    }
    best.map(|(_, id)| (id, args))
}

/// Handle `/skillname` activation. Returns `Some(quit)` if handled, `None` to continue.
async fn handle_skill_activation(
    app: &mut App,
    input: &str,
    terminal: &mut Terminal<CrosstermBackend<std::io::Stdout>>,
) -> Option<bool> {
    let trimmed = input.trim();
    let rest = trimmed.strip_prefix('/')?;
    let (name, user_args) = if let Some(space_pos) = rest.find(' ') {
        (&rest[..space_pos], rest[space_pos + 1..].trim())
    } else {
        (rest.trim(), "")
    };
    if name.is_empty() {
        return None;
    }

    // Look up skill by name
    let skill = app.config.skills.iter().find(|s| s.name == name);
    if let Some(skill) = skill {
        // Read SKILL.md and inject as user message
        let content = match std::fs::read_to_string(&skill.file_path) {
            Ok(c) => c,
            Err(e) => {
                app.add_system_message(&format!("Could not read skill \"{name}\": {e}"));
                app.chat.auto_scroll = true;
                return Some(false);
            }
        };

        let user_input = if user_args.is_empty() {
            format!("Skill: {}\n\n{}", name, content.trim())
        } else {
            format!(
                "Skill: {}\n\n{}\n\nUser: {}",
                name,
                content.trim(),
                user_args
            )
        };

        // Snapshot pre-turn skills before activation so a cancelled
        // deferred turn restores the exact pre-turn set.
        let pre_turn_skills = app.active_skills.clone();

        // Activate the skill (in-memory set + session header, so the header
        // stays the source of truth for compaction re-injection).
        app.activate_skill(name);
        app.session.session_store.add_activated_skill(name);

        // Delegate to run_turn_with_input
        // Display only the original user input (e.g. "/refine some prompt") in chat.
        // LLM receives the full SKILL.md + prompt.
        match run_turn_with_input(
            app,
            user_input,
            Some(input.to_string()),
            pre_turn_skills,
            terminal,
        )
        .await
        {
            Ok(quit) => Some(quit),
            Err(e) => {
                app.add_system_message(&format!("Skill execution failed: {e}"));
                app.chat.auto_scroll = true;
                Some(false)
            }
        }
    } else {
        // Skill not found. No palette-command fallback: if `name` matched a
        // command, `parse_slash_command` in `start_turn` would have handled
        // it before this function ran.
        let available: Vec<&str> = app.config.skills.iter().map(|s| s.name.as_str()).collect();
        let msg = if available.is_empty() {
            format!("Skill \"{name}\" not found. No skills loaded.")
        } else {
            format!(
                "Skill \"{}\" not found. Available skills: {}",
                name,
                available.join(", ")
            )
        };
        app.add_system_message(&msg);
        app.chat.auto_scroll = true;
        Some(false)
    }
}

/// Run a turn with a given input string.
/// Used by `start_turn` and by commands that generate their own input (e.g. `/init`).
///
/// `display_text` controls what appears in the chat area. When `None`, `input` is used
/// as the display text (backward compatible). When `Some(text)`, that text is shown to
/// the user while `input` is sent to the LLM.
///
/// # Errors
///
/// - Hook execution errors (exit code != 0).
/// - Agent streaming errors (network, serialization, context overflow).
/// - Session persistence errors (file I/O).
#[expect(
    clippy::implicit_hasher,
    reason = "skill-name set; the default hasher is the only sensible representation"
)]
pub async fn run_turn_with_input(
    app: &mut App,
    input: String,
    display_text: Option<String>,
    pre_turn_skills: HashSet<String>,
    terminal: &mut Terminal<CrosstermBackend<std::io::Stdout>>,
) -> anyhow::Result<bool> {
    // @path references are sent literally to the model.
    // The model uses `read` for files and `ls` for directories.

    // In-progress guard: a compaction task is already running. Starting a
    // turn now would spawn a second compaction task, and the finished plan
    // could later be applied to a replaced session store.
    if app.compaction_task.is_some() {
        app.notify_compaction_in_progress();
        return Ok(false);
    }

    // Snapshot for rollback if this turn defers to background compaction
    // and the user cancels it.
    let pre_turn_chat_len = app.chat.chat_messages.len();

    // Run UserPromptSubmit hooks (before adding user message). A blocked
    // hook (exit 2) stops the turn; successful stdout is a side effect and
    // never reaches the model.
    let (prompt_blocked, blocked_error, _ups_outputs) = crate::hooks::run_tui_hooks(
        app,
        crate::hooks::HookEvent::UserPromptSubmit,
        Some(input.clone()),
        None,
    )
    .await;

    if prompt_blocked {
        app.add_system_message(&format!("Prompt blocked by hook: {blocked_error}"));
        app.chat.auto_scroll = true;
        return Ok(false);
    }

    // The model receives exactly the user's text — hook stdout is a side
    // effect and never reaches the model, so `model_input` is the input itself.
    let model_input = input.clone();

    // Display text: what the user sees in chat and what the editor is
    // restored to on cancel. `None` → the input itself.
    let display = display_text.unwrap_or_else(|| input.clone());

    // Add user message to chat (no hook context XML tags — hook output is displayed as HookMessage)
    app.add_user_message(&display);
    app.chat.auto_scroll = true;

    // Auto-create session if none exists.
    // Do NOT append a system message to the session — the system prompt
    // is injected by build_context_with_system() to avoid duplicate
    // system messages that break Jinja templates requiring system at pos 0.
    let created_session = !app.session.session_store.has_active_session();
    if created_session {
        app.session.session_store.create_session(
            app.config.model_name.clone(),
            app.config.context_window,
            app.config.cwd.to_string_lossy().to_string(),
        )?;
        // Fresh session — the history guard must not compare against the
        // previous session's baseline.
        app.session.reset_history_guard();
        // A skill activated before the session existed (e.g. a `/skillname`
        // first message) was not persisted by `add_activated_skill` (no
        // header yet). Sync the header so it stays the source of truth for
        // compaction re-injection.
        for name in &app.active_skills {
            app.session.session_store.add_activated_skill(name);
        }
    }

    // Pre-prompt compaction check: if context is near the threshold, compact before sending
    {
        let compaction_settings = CompactionSettings::from_config(&app.config.config.compaction);

        // Estimate system overhead from the cached prompt (built at the last
        // turn start; first turn builds it here) — no per-check disk read (H2).
        let est = app.session.session_store.cached_estimator();
        let system_overhead = app
            .session
            .session_store
            .system_prompt_tokens(&app.config.cwd, &app.config.skills);

        // Use estimate_context_tokens which prefers actual LLM usage data
        let estimated = app
            .session
            .session_store
            .estimate_context_tokens(system_overhead);
        let user_msg_tokens = est.estimate_tokens(&model_input);
        let total_with_input = estimated.saturating_add(user_msg_tokens);

        if compaction_settings.should_compact(total_with_input, app.config.context_window) {
            // Run PreCompact hooks before deferring. A blocked hook (exit 2)
            // vetoes compaction: undo the turn setup and do not launch the
            // agent (mirrors the mid-turn "don't proceed" semantics).
            let (precompact_blocked, precompact_blocked_error, precompact_outputs) =
                crate::hooks::run_tui_hooks(app, crate::hooks::HookEvent::PreCompact, None, None)
                    .await;

            if precompact_blocked {
                undo_turn_setup(
                    app,
                    pre_turn_chat_len,
                    created_session,
                    display,
                    pre_turn_skills,
                );
                // Added after the truncation so the block note survives.
                app.add_system_message(&format!(
                    "Compaction blocked by hook: {precompact_blocked_error}"
                ));
                app.chat.auto_scroll = true;
                return Ok(false);
            }

            // Capture PreCompact hook stdout for the summary (applied in
            // `handle_compaction_result`).
            let precompact_context = crate::hooks::format_hook_context(&precompact_outputs);
            if !precompact_context.is_empty() {
                app.pending_precompact_context = Some(precompact_context);
            }

            app.add_system_message("Context approaching limit. Compacting before sending...");
            app.chat.auto_scroll = true;

            // Defer the turn: spawn compaction into the main loop's task
            // slot. `event_loop::poll_compaction_task` applies the result
            // when it finishes and launches the agent, keeping the UI
            // responsive in the meantime (sweep animation, scroll, and
            // Esc-to-cancel with rollback via `rollback_pending_turn`).
            // A failed compaction still launches the turn — the agent
            // loop's overflow path is the fallback.
            app.modal.state = AppState::Compacting;

            let entries = app.session.session_store.entries().to_vec();
            let engine = crate::session::CompactionEngine::from_config(&app.config.config);
            let provider = crate::agent::build_provider(&app.config.config)?;
            let config = app.config.config.clone();
            let cwd = app.config.cwd.clone();
            let skills = app.config.skills.clone();

            // Auto compaction (pre-prompt) — the PostCompact hook reports
            // trigger = "auto".
            app.compaction_trigger = "auto";
            app.compaction_task = Some(tokio::spawn(async move {
                engine
                    .compute_compaction(provider, config, entries, cwd, skills, None)
                    .await
            }));

            app.pending_turn = Some(PendingTurn {
                input: model_input.clone(),
                restore_input: display,
                created_session,
                pre_turn_chat_len,
                pre_turn_skills,
            });
            return Ok(false);
        }
    }

    launch_agent_turn(app, model_input, terminal).await
}

/// Roll back a deferred turn after compaction cancellation.
///
/// Truncates chat to the pre-turn length, discards an auto-created
/// session, restores active skills, and puts the typed text back in the
/// editor. No-op if no turn is pending.
pub fn rollback_pending_turn(app: &mut App) {
    let Some(pending) = app.pending_turn.take() else {
        return;
    };
    undo_turn_setup(
        app,
        pending.pre_turn_chat_len,
        pending.created_session,
        pending.restore_input,
        pending.pre_turn_skills,
    );
}

/// Undo the setup a turn performed before it was launched: truncate chat to
/// the pre-turn length, discard an auto-created session, restore active
/// skills, and put the typed text back in the editor. Also clears any
/// `PreCompact` hook context captured for a compaction that never ran (the
/// cancel path bypasses `handle_compaction_result`, the only other clear).
///
/// Shared by `rollback_pending_turn` (compaction cancel) and the blocked
/// `PreCompact` pre-prompt path (a hook vetoed compaction, so the turn must
/// not proceed).
fn undo_turn_setup(
    app: &mut App,
    pre_turn_chat_len: usize,
    created_session: bool,
    restore_input: String,
    pre_turn_skills: HashSet<String>,
) {
    app.chat.chat_messages.truncate(pre_turn_chat_len);
    // The compaction this context was captured for will not run; drop it so
    // it can't leak into a later compaction's summary.
    app.pending_precompact_context = None;
    if created_session {
        // The auto-created session file is empty (no messages were saved before
        // the turn was rolled back); remove it so orphaned empty `.jsonl` files
        // don't accumulate. The store is then replaced with a fresh one.
        if let Some(path) = app.session.session_store.current_session_path.clone() {
            let _ = std::fs::remove_file(&path);
        }
        app.session.session_store = crate::session::SessionStore::new(&app.config.cwd);
        app.session.reset_history_guard();
    }
    app.active_skills = pre_turn_skills;
    app.editor.input_buffer = restore_input;
    app.editor.cursor_pos = app.editor.input_buffer.len();
}

/// Create the SSE debug capture guard for a turn, if capture is enabled.
///
/// The guard holds the global `CURRENT_TURN` slot; the caller must keep it
/// alive for the duration of the turn (the provider reads the capture
/// during every request). `None` when SSE debug is not enabled.
#[must_use]
pub(crate) fn sse_turn_guard(
    config: Option<&crate::sse_debug::SseDebugConfig>,
    session_id: Option<&str>,
) -> Option<crate::sse_debug::TurnCaptureGuard> {
    config
        .and_then(|config| config.start_turn(session_id))
        .map(crate::sse_debug::set_current_turn)
}

/// Result of [`restart_agent`]: the spawned agent task and its channels.
///
/// The agent task holds its own clones of the event sender and cancel
/// receiver; the caller only needs the receiver side. `sse_guard` must be
/// held by the caller until the end of the turn (the provider writes
/// captures during every request of the task).
pub(crate) struct RestartedAgent {
    /// The spawned agent task.
    pub handle: tokio::task::JoinHandle<()>,
    /// The event receiver for the TUI event loop.
    pub rx: tokio::sync::mpsc::Receiver<AgentEvent>,
    /// The cancel sender (the agent task holds its own receiver clone).
    pub cancel_tx: tokio::sync::watch::Sender<bool>,
    /// The SSE capture guard, if capture is enabled.
    pub sse_guard: Option<crate::sse_debug::TurnCaptureGuard>,
}

/// Core of the agent restart: the 12 steps shared by every launch/restart
/// site (M1).
///
/// Owns: `start_streaming`, tracker start, `reset_request_usage`, the
/// SSE-debug guard, `build_context_with_system` + AGENTS.md warning,
/// `append_message`, channel setup, the state clones, and the `spawn`.
pub(crate) fn restart_agent(app: &mut App, user_msg: String) -> RestartedAgent {
    app.start_streaming(&user_msg);
    app.agent
        .tracker
        .start_with_estimator(&app.session.session_store.cached_estimator());
    app.reset_request_usage();

    // Set up SSE debug capture for this turn (if enabled via RESPONDAMI_SSE_DEBUG)
    let turn_capture_guard = sse_turn_guard(
        crate::logging::sse_debug_config(),
        app.session.session_store.session_id(),
    );

    // Build context messages from session_store (which stays in app)
    let (context_messages, agents_md_error) = crate::agent::build_context_with_system(
        &mut app.session.session_store,
        &app.config.cwd,
        &app.config.skills,
    );

    // Show warning if AGENTS.md couldn't be loaded
    if let Some(err) = agents_md_error {
        app.add_system_message(&format!("⚠ Could not load AGENTS.md: {err}"));
        app.chat.auto_scroll = true;
    }

    // Save user message to session (session_store stays in app)
    if app.session.session_store.has_active_session() {
        let parent_id = app.session.session_store.last_entry_id();
        if let Err(e) = app.session.session_store.append_message(
            parent_id,
            crate::session::AgentMessage::user(user_msg.clone()),
        ) {
            tracing::error!("Failed to save user message: {e}");
        }
    }

    let (tx, rx) = tokio::sync::mpsc::channel::<AgentEvent>(256);
    let (cancel_tx, cancel_rx) = tokio::sync::watch::channel(false);
    let config = app.config.config.clone();
    let cwd = app.config.cwd.clone();
    let agent_tool_registry = app.tool_registry.clone();
    let rtk_state = app.config.rtk_state.clone();
    let hook_registry = app.config.hook_registry.clone();
    let skills = app.config.skills.clone();
    let history_guard = app.session.history_guard.clone();

    let agent_tx = tx.clone();
    let agent_cancel_rx = cancel_rx.clone();
    let agent_handle = tokio::spawn(async move {
        crate::agent::run_agent_with_snapshot(
            context_messages,
            user_msg,
            config,
            cwd,
            agent_tool_registry,
            hook_registry,
            skills,
            agent_tx,
            agent_cancel_rx,
            rtk_state,
            history_guard,
        )
        .await;
    });

    RestartedAgent {
        handle: agent_handle,
        rx,
        cancel_tx,
        sse_guard: turn_capture_guard,
    }
}

/// Launch the agent for a turn whose input is already displayed in chat.
///
/// Runs the agent loop in a background task and processes its events until
/// the turn completes (or the user quits).
///
/// # Errors
///
/// - Session persistence errors (file I/O).
/// - Agent streaming errors (network, serialization, context overflow).
pub async fn launch_agent_turn(
    app: &mut App,
    user_msg: String,
    terminal: &mut Terminal<CrosstermBackend<std::io::Stdout>>,
) -> anyhow::Result<bool> {
    let agent = restart_agent(app, user_msg);

    // Process agent events while maintaining TUI responsiveness
    process_agent_events(app, agent.handle, agent.cancel_tx, agent.rx, terminal).await;

    app.modal.state = AppState::Idle;
    Ok(false)
}
