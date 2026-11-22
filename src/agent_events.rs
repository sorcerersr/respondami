//! Agent event processing — bridges the async agent loop with the TUI.
//!
//! Drains `AgentEvent`s from the channel while keeping the terminal responsive.
//! Handles streaming tokens, reasoning, tool calls, usage stats, compaction
//! triggers, and cooperative cancellation. Implements the drain-on-done pattern
//! to prevent final output lines from being silently dropped.

use std::time::Duration;

use crossterm::event::{self, Event, KeyEventKind, MouseEventKind};
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;

use std::fmt;

use crate::agent::build_provider;
use crate::session::{CompactionEngine, CompactionPlan};
use crate::tui::{AbortReason, AgentEvent, App, AppState, CompactionReason, Theme};

/// Pending auto-compaction task and associated metadata.
///
/// Holds the background `compute_compaction` task along with the retry message
/// and reason needed to restart the agent after compaction completes.
struct PendingCompaction {
    handle: tokio::task::JoinHandle<anyhow::Result<CompactionPlan>>,
    retry_user_message: Option<String>,
    reason: CompactionReason,
}

impl fmt::Debug for PendingCompaction {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PendingCompaction")
            .field("handle", &self.handle.is_finished())
            .field(
                "retry_user_message",
                &self.retry_user_message.as_ref().map(String::len),
            )
            .field("reason", &self.reason)
            .finish()
    }
}

/// Process agent events while maintaining TUI responsiveness.
/// Uses `EventStream` for terminal events (same as main loop).
/// Uses cooperative cancellation via watch channel (no abort).
///
/// `agent_handle` is the spawned agent task. On cancellation, we await this handle
/// with a timeout to ensure the agent finishes cleanly before returning.
pub async fn process_agent_events(
    app: &mut App,
    mut agent_handle: tokio::task::JoinHandle<()>,
    mut cancel_tx: tokio::sync::watch::Sender<bool>,
    mut rx: tokio::sync::mpsc::Receiver<AgentEvent>,
    terminal: &mut Terminal<CrosstermBackend<std::io::Stdout>>,
) {
    let mut done = false;
    let theme = Theme::gh_dark();
    // Pending auto-compaction task (spawn-and-poll pattern).
    // When Some, the event loop polls this task instead of blocking on .await.
    let mut pending_compaction: Option<PendingCompaction> = None;
    // SSE debug capture guard for the current agent task (M1: restart_agent
    // creates it; held until the event loop ends so the provider can write
    // captures for every request of the turn).
    let mut sse_guard: Option<crate::sse_debug::TurnCaptureGuard> = None;
    while !done {
        // Draw the frame
        if crate::event_loop::draw_frame(app, terminal, &theme).is_err() {
            break;
        }

        // Tick activity indicator animation (time-based, ~10 ticks/s)
        crate::event_loop::tick_activity_indicator(app, &theme);

        // Poll in-flight auto-compaction task.
        // When a compaction task is running, the event loop continues to draw,
        // tick the animation, and process terminal events — the UI stays responsive.
        if let Some(pending) = pending_compaction.as_mut()
            && pending.handle.is_finished()
        {
            let compaction = pending_compaction.take().unwrap();
            let retry_user_message = compaction.retry_user_message;
            let reason = compaction.reason;
            let handle = compaction.handle;

            match crate::event_loop::handle_compaction_result(app, handle, true).await {
                crate::event_loop::CompactionResult::Success { .. } => {
                    // Restart agent after compaction (overflow recovery or threshold auto-continue).
                    if let Some(user_msg) = retry_user_message {
                        let system_msg = match reason {
                            CompactionReason::Overflow => {
                                "Context overflow recovered. Retrying your message..."
                            }
                            CompactionReason::Threshold => "Context compacted. Continuing task...",
                        };
                        app.add_system_message(system_msg);
                        app.maybe_auto_scroll();

                        let new_agent = crate::turn::restart_agent(app, user_msg);
                        sse_guard = new_agent.sse_guard;

                        agent_handle = new_agent.handle;

                        // Cancel the old cancel channel
                        let _ = cancel_tx.send(true);

                        // Replace channels and continue event loop
                        rx = new_agent.rx;
                        cancel_tx = new_agent.cancel_tx;
                        // Fall through to continue the loop (don't set done = true)
                    }
                }
                crate::event_loop::CompactionResult::Failed(_)
                | crate::event_loop::CompactionResult::Panicked => {
                    crate::event_loop::transition_to_idle(app);
                    done = true;
                }
            }
        }

        // Process agent events (non-blocking with short timeout)
        match tokio::time::timeout(Duration::from_millis(20), rx.recv()).await {
            Ok(Some(AgentEvent::Token(token))) => {
                // Transition from ToolExec back to Streaming when the next
                // response phase begins (after tool calls, the model streams
                // again before deciding whether to call more tools).
                if app.modal.state == AppState::ToolExec {
                    app.modal.state = AppState::Streaming;
                }
                app.agent.tracker.record(token.len());
                app.push_token(&token);
                app.maybe_auto_scroll();
            }
            Ok(Some(AgentEvent::ThinkingStart)) => {
                // Transition from ToolExec back to Streaming — thinking is
                // part of the streaming response phase.
                if app.modal.state == AppState::ToolExec {
                    app.modal.state = AppState::Streaming;
                }
                app.add_thinking_message();
                app.maybe_auto_scroll();
            }
            Ok(Some(AgentEvent::ThinkingEnd)) => {
                // Thinking block is static — no animation to stop
            }
            Ok(Some(AgentEvent::Reasoning(text))) => {
                app.agent.tracker.record(text.len());
                app.push_reasoning(&text);
                app.maybe_auto_scroll();
            }
            Ok(Some(AgentEvent::ToolCallStart {
                tool_call_id,
                tool_name,
                tool_args,
            })) => {
                app.modal.state = AppState::ToolExec;
                app.add_pending_tool_call(&tool_call_id, &tool_name, &tool_args);
                app.maybe_auto_scroll();
            }
            Ok(Some(AgentEvent::ToolCallDone {
                tool_call_id,
                result,
                has_error,
                cancelled,
                rtk_original,
            })) => {
                app.agent.tracker.pause_timing(); // Exclude tool exec time from streaming time
                app.update_tool_call_result(
                    &tool_call_id,
                    &result,
                    has_error,
                    cancelled,
                    rtk_original,
                );
                // Loop-recovery counter decay: after enough loop-free tool
                // calls, a distinct later loop gets a fresh budget (long
                // sessions). Only meaningful once a recovery has happened.
                app.apply_loop_decay();
                app.maybe_auto_scroll();
            }
            Ok(Some(AgentEvent::ToolOutput(text))) => {
                app.append_tool_output(&text);
                app.maybe_auto_scroll();
            }
            Ok(Some(AgentEvent::RetryStart {
                attempt,
                max_attempts,
                delay_ms,
                error,
            })) => {
                app.add_system_message(&format!(
                    "Retrying... (attempt {}/{}, {}ms delay) — {}",
                    attempt + 1,
                    max_attempts,
                    delay_ms,
                    error
                ));
                app.maybe_auto_scroll();
            }
            Ok(Some(AgentEvent::RetryEnd { success, attempt })) => {
                if success {
                    app.add_system_message(&format!("Retry succeeded after {attempt} attempt(s)."));
                } else {
                    app.add_system_message(&format!("Retry failed after {attempt} attempt(s)."));
                }
                app.maybe_auto_scroll();
            }
            Ok(Some(AgentEvent::Usage(usage))) => {
                app.accumulate_token_usage(&usage);
                app.agent
                    .tracker
                    .apply_provider_usage(usage.completion_tokens);
            }
            Ok(Some(AgentEvent::Compaction {
                tokens_saved,
                message_count,
            })) => {
                app.add_compaction_message(tokens_saved, message_count);
                app.maybe_auto_scroll();
            }
            Ok(Some(AgentEvent::CompactionProgress { message })) => {
                app.add_system_message(&message);
                app.maybe_auto_scroll();
            }
            Ok(Some(AgentEvent::CompactionDone {
                tokens_before,
                tokens_after,
                messages_removed,
            })) => {
                app.add_compaction_message(
                    tokens_before.saturating_sub(tokens_after),
                    messages_removed,
                );
                app.maybe_auto_scroll();
            }
            Ok(Some(AgentEvent::CompactionError { message })) => {
                app.add_system_message(&format!("Compaction failed: {message}"));
                app.maybe_auto_scroll();
            }
            Ok(Some(AgentEvent::SaveAssistantMessage { content, usage })) => {
                if app.session.session_store.has_active_session() {
                    let parent_id = app.session.session_store.last_entry_id();
                    if let Err(e) = app.session.session_store.append_message(
                        parent_id,
                        crate::session::AgentMessage::assistant_with_blocks(content, usage),
                    ) {
                        tracing::error!("Failed to save assistant message: {}", e);
                    }
                }
            }
            Ok(Some(AgentEvent::SaveToolResult {
                tool_call_id,
                tool_name,
                tool_args,
                result,
                images,
            })) => {
                if app.session.session_store.has_active_session() {
                    let parent_id = app.session.session_store.last_entry_id();
                    if let Err(e) = app.session.session_store.append_message(
                        parent_id,
                        crate::session::AgentMessage::tool(
                            tool_call_id,
                            tool_name,
                            tool_args,
                            result,
                            images,
                        ),
                    ) {
                        tracing::error!("Failed to save tool result: {}", e);
                    }
                }
            }
            Ok(Some(AgentEvent::StripImages { tool_call_ids })) => {
                handle_strip_images_event(app, tool_call_ids);
            }
            Ok(Some(AgentEvent::HookMessage {
                event,
                hook_name,
                success,
                stdout,
                stderr,
                tool_name,
            })) => {
                // Drop hook messages when display mode is Hidden
                if app.config.hook_display != crate::tui::HookDisplay::Hidden {
                    app.chat.chat_messages.push(crate::tui::ChatMessage::Hook(
                        crate::tui::messages::HookMessage {
                            event,
                            hook_name,
                            success,
                            stdout,
                            stderr,
                            tool_name,
                        },
                    ));
                    app.maybe_auto_scroll();
                }
            }
            Ok(Some(AgentEvent::SkillActivation { skill_name })) => {
                handle_skill_activation_event(app, &skill_name);
            }
            Ok(Some(AgentEvent::NeedsCompaction {
                reason,
                retry_user_message,
                ..
            })) => {
                // Drain remaining events from the channel before compaction
                app.drain_pending_events(&mut rx);
                if let Some(snap) = app.agent.tracker.finalize_turn() {
                    app.session.session_store.save_token_rate(snap);
                }

                // Run PreCompact hooks before compaction. A blocked hook
                // (exit 2) stops the turn; successful stdout is appended to
                // the summary in `handle_compaction_result`.
                let (compaction_blocked, compaction_blocked_error, precompact_outputs) =
                    crate::hooks::run_tui_hooks(
                        app,
                        crate::hooks::HookEvent::PreCompact,
                        None,
                        None,
                    )
                    .await;

                if compaction_blocked {
                    app.add_system_message(&format!(
                        "Compaction blocked by hook: {compaction_blocked_error}"
                    ));
                    app.maybe_auto_scroll();
                    app.modal.state = AppState::Idle;
                    done = true;
                    continue;
                }

                // Capture PreCompact hook stdout for the summary (applied in
                // `handle_compaction_result`).
                let precompact_context = crate::hooks::format_hook_context(&precompact_outputs);
                if !precompact_context.is_empty() {
                    app.pending_precompact_context = Some(precompact_context);
                }

                // Spawn compaction LLM call as a background task (spawn-and-poll pattern).
                // This keeps the event loop responsive — the UI draws, animates, and
                // processes terminal events while compaction runs.
                let live_config = app.config.live_config();
                let engine = CompactionEngine::from_config(&live_config);
                let provider = match build_provider(&live_config) {
                    Ok(p) => p,
                    Err(e) => {
                        app.add_system_message(&format!(
                            "Compaction failed: Failed to build provider: {e}"
                        ));
                        app.maybe_auto_scroll();
                        app.modal.state = AppState::Idle;
                        done = true;
                        continue;
                    }
                };
                let entries = app.session.session_store.entries().to_vec();
                let config = live_config;
                let cwd = app.config.cwd.clone();
                let skills = app.config.skills.clone();

                let handle = tokio::spawn(async move {
                    engine
                        .compute_compaction(provider, config, entries, cwd, skills, None)
                        .await
                });

                pending_compaction = Some(PendingCompaction {
                    handle,
                    retry_user_message,
                    reason,
                });
                app.modal.state = AppState::Compacting;
                continue;
            }
            Ok(Some(AgentEvent::LoopDetected { report })) => {
                // A degenerate loop was detected; the agent task has stopped
                // (no `Done` follows, so the Stop hook never fires). Decide:
                // cut the looped history and restart (within budget) or stop.
                app.drain_pending_events(&mut rx);
                if let Some(snap) = app.agent.tracker.finalize_turn() {
                    app.session.session_store.save_token_rate(snap);
                }
                app.loop_free_tool_calls = 0;

                match apply_loop_recovery(app, &report) {
                    Some(new_agent) => {
                        sse_guard = new_agent.sse_guard;
                        agent_handle = new_agent.handle;
                        let _ = cancel_tx.send(true);
                        rx = new_agent.rx;
                        cancel_tx = new_agent.cancel_tx;
                    }
                    None => done = true,
                }
                continue;
            }
            Ok(Some(AgentEvent::Done(Ok(())))) => {
                // Drain any remaining Token/Reasoning/Usage events from the channel.
                // Done may arrive while tokens are still buffered (channel cap 256).
                app.drain_pending_events(&mut rx);
                if let Some(snap) = app.agent.tracker.finalize_turn() {
                    app.session.session_store.save_token_rate(snap);
                }

                // Text-echo loop check before the Stop hook: a text-only turn
                // that repeats the same final response is a degenerate loop
                // (typically a Stop-hook re-prompt cycle). A trip takes the
                // same recovery path as a tool-call loop and skips the Stop
                // hook; a turn with tool activity is progress and clears the
                // detector's window.
                if app.config.config.loop_detection.enabled {
                    if app.agent.pending_tool_calls.is_empty() {
                        if let Some(report) = app
                            .text_loop_detector
                            .record(&last_assistant_text(app))
                        {
                            app.loop_free_tool_calls = 0;
                            match apply_loop_recovery(app, &report) {
                                Some(new_agent) => {
                                    // The cut removed the looped history, so the
                                    // detector's evidence is gone: reset the window
                                    // or the next response (even a different one)
                                    // re-trips on the stale key and the model can
                                    // never escape by taking a different action.
                                    app.text_loop_detector.reset();
                                    sse_guard = new_agent.sse_guard;
                                    agent_handle = new_agent.handle;
                                    let _ = cancel_tx.send(true);
                                    rx = new_agent.rx;
                                    cancel_tx = new_agent.cancel_tx;
                                }
                                None => done = true,
                            }
                            continue;
                        }
                    } else {
                        app.text_loop_detector.note_tool_activity();
                    }
                }

                // Run Stop hooks before finishing. A blocked hook (exit 2)
                // restarts the agent with the stderr as its next instruction.
                let (stop_blocked, stop_blocked_error, _stop_outputs) =
                    crate::hooks::run_tui_hooks(app, crate::hooks::HookEvent::Stop, None, None)
                        .await;

                if stop_blocked {
                    // Feed stderr back to the model as its next instruction
                    app.add_system_message(&format!(
                        "Agent blocked from stopping: {stop_blocked_error}"
                    ));
                    app.maybe_auto_scroll();
                    // Restart the agent with the error as a user message
                    let wrapped_error = format!(
                        "Agent was blocked from stopping. Reason: {stop_blocked_error}\n\nContinue your current work.",
                    );

                    let new_agent = crate::turn::restart_agent(app, wrapped_error);
                    sse_guard = new_agent.sse_guard;

                    agent_handle = new_agent.handle;

                    // Cancel the old cancel channel
                    let _ = cancel_tx.send(true);

                    // Replace channels and continue event loop
                    rx = new_agent.rx;
                    cancel_tx = new_agent.cancel_tx;
                    continue;
                }

                // Snap viewport to bottom so the complete response is visible.
                app.maybe_auto_scroll();
                done = true;
            }
            Ok(Some(AgentEvent::Done(Err(AbortReason::UserCancelled)))) => {
                // User cancelled — drain remaining events (including Usage), show "Cancelled" message.
                app.drain_pending_events(&mut rx);
                if let Some(snap) = app.agent.tracker.finalize_turn() {
                    app.session.session_store.save_token_rate(snap);
                }
                app.add_system_message("Cancelled.");
                app.maybe_auto_scroll();
                done = true;
            }
            Ok(Some(AgentEvent::Done(Err(AbortReason::StreamAborted)))) => {
                // HTTP-level abort — treat same as user cancellation.
                app.drain_pending_events(&mut rx);
                if let Some(snap) = app.agent.tracker.finalize_turn() {
                    app.session.session_store.save_token_rate(snap);
                }
                app.add_system_message("Cancelled.");
                app.maybe_auto_scroll();
                done = true;
            }
            Ok(Some(AgentEvent::Done(Err(AbortReason::Error(e))))) => {
                // Real error — drain remaining events (including Usage).
                // Done may arrive while tokens are still buffered.
                app.drain_pending_events(&mut rx);
                if let Some(snap) = app.agent.tracker.finalize_turn() {
                    app.session.session_store.save_token_rate(snap);
                }
                app.add_system_message(&format!("Agent error: {e}"));
                app.maybe_auto_scroll();
                done = true;
            }
            Ok(None) => {
                if pending_compaction.is_some() {
                    // Agent channel closed but compaction task still running.
                    // Sleep to avoid tight loop on closed channel, continue polling.
                    tokio::time::sleep(Duration::from_millis(20)).await;
                    continue;
                }
                app.maybe_auto_scroll();
                done = true;
            }
            Err(_) => {}
        }

        // Drain ALL pending terminal events (non-blocking).
        // During streaming, main EventStream is idle — no competition.
        if event::poll(Duration::from_millis(0)).unwrap_or(false) {
            while event::poll(Duration::from_millis(0)).unwrap_or(false) {
                match event::read() {
                    Ok(Event::Key(key)) if key.kind == KeyEventKind::Press => {
                        match key.code {
                            crossterm::event::KeyCode::PageUp => {
                                let page = get_chat_visible_height(app);
                                app.chat.scroll_up(page);
                            }
                            crossterm::event::KeyCode::PageDown => {
                                let page = get_chat_visible_height(app);
                                app.chat.scroll_down(page);
                            }
                            _ if pending_compaction.is_some()
                                && ((key.code == crossterm::event::KeyCode::Char('c')
                                    && key
                                        .modifiers
                                        .contains(crossterm::event::KeyModifiers::CONTROL))
                                    || key.code == crossterm::event::KeyCode::Esc) =>
                            {
                                // Abort the background compaction task
                                if let Some(pending) = pending_compaction.take() {
                                    pending.handle.abort();
                                }
                                app.add_system_message("Compaction cancelled.");
                                app.chat.auto_scroll = true;
                                app.modal.state = AppState::Idle;
                                done = true;
                            }
                            _ if app.is_working()
                                && ((key.code == crossterm::event::KeyCode::Char('c')
                                    && key
                                        .modifiers
                                        .contains(crossterm::event::KeyModifiers::CONTROL))
                                    || key.code == crossterm::event::KeyCode::Esc) =>
                            {
                                // Signal cooperative cancellation
                                let _ = cancel_tx.send(true);
                                app.add_system_message("Cancelling... (Ctrl+D to force quit)");
                                app.chat.auto_scroll = true;
                                // Don't set done yet — wait for the agent to finish
                                // The Done event will be sent when the agent finishes
                            }
                            _ if crate::key_handler::layers::streaming_ui::handle_ui_shortcuts(
                                app, &key,
                            )
                            .unwrap_or(false) =>
                            {
                                // UI shortcut handled (F1, Ctrl+O, Ctrl+T)
                            }
                            _ => {}
                        }
                    }
                    Ok(Event::Mouse(mouse)) => {
                        match mouse.kind {
                            MouseEventKind::ScrollUp => {
                                app.chat.scroll_up(crate::mouse::SCROLL_LINES);
                            }
                            MouseEventKind::ScrollDown => {
                                app.chat.scroll_down(crate::mouse::SCROLL_LINES);
                            }
                            _ => {} // clicks, drags, moves — ignored
                        }
                    }
                    Ok(Event::Resize(_, _)) => {
                        let _ = terminal.clear();
                    }
                    _ => {}
                }
            }
        }
    }

    // Agent task finished (naturally or via cooperative cancel).
    // Await the agent task with a timeout to ensure it's fully cleaned up.
    // This prevents task leaks and ensures all resources are freed.
    let _ = tokio::time::timeout(Duration::from_secs(5), agent_handle).await;
    // The SSE capture guard lives until the very end of the turn (the
    // provider writes captures during every request of the agent task).
    drop(sse_guard);
}

/// Handle a `SkillActivation` event: update the active skills set, register
/// the skill's frontmatter hooks (idempotent — a re-activation refreshes them
/// from the skill file; activation-scoped firing), persist the activation to
/// the session, and display the activation message.
pub(crate) fn handle_skill_activation_event(app: &mut App, skill_name: &str) {
    app.active_skills.insert(skill_name.to_string());
    if let Some(skill) = app.config.skills.iter().find(|s| s.name == skill_name) {
        crate::hooks::register_skill_hooks(&mut app.config.hook_registry, skill);
    }
    // Persist to session
    app.session.session_store.add_activated_skill(skill_name);
    // Display the skill activation message (only this, no system message)
    app.chat
        .chat_messages
        .push(crate::tui::ChatMessage::SkillActivation(
            crate::tui::messages::SkillActivationMessage {
                skill_name: skill_name.to_string(),
            },
        ));
    app.maybe_auto_scroll();
}

/// `StripImages` handler (`strip_images_after_view` workaround): the agent
/// loop already stripped the live context; apply the identical transform to
/// the session and rewrite the JSONL (atomic). Failure is logged, not fatal
/// — the live session stays correct in memory; a later resume re-strips
/// (one re-prefill).
pub(crate) fn handle_strip_images_event(app: &mut App, tool_call_ids: Vec<String>) {
    if app.session.session_store.has_active_session()
        && let Err(e) = app.session.session_store.strip_tool_images(&tool_call_ids)
    {
        tracing::error!("Failed to strip images from session: {e}");
    }
}

/// The outcome of a [`decide_loop_recovery`] decision.
#[derive(Debug)]
pub(crate) enum LoopRecoveryDecision {
    /// The cut was applied (or there was no session to cut); restart the agent
    /// with `prompt`. `removed` is the number of messages truncated.
    Recover { prompt: String, removed: u32 },
    /// The recovery budget is exhausted: stop, no cut.
    Exhausted,
    /// The cut failed (I/O): stop with the error message.
    Failed(String),
}

/// Decide how to recover from a detected loop: apply the surgical cut and
/// build the recovery prompt, or stop (budget exhausted / cut failed).
///
/// This is the pure decision core of the loop-recovery path — separated from
/// the channel swapping so it can be unit-tested. It mutates the session
/// (the cut), the history guard (reset), and the recovery counter (increment).
/// The caller handles the system message, `restart_agent`, and channel swap.
///
/// The cut point and prompt depend on the loop kind: a tool-call loop cuts
/// at the first assistant message that issued the repeated call; a text-echo
/// loop cuts at the user message preceding the first matching response.
pub(crate) fn decide_loop_recovery(
    app: &mut App,
    report: &crate::agent::LoopReport,
) -> LoopRecoveryDecision {
    let max_recoveries = app.config.config.loop_detection.max_recoveries;
    if app.loop_recovery_count >= max_recoveries {
        return LoopRecoveryDecision::Exhausted;
    }

    // Locate and apply the surgical cut. The pre-loop work and the user's
    // instruction are preserved.
    let entries = app.session.session_store.entries().to_vec();
    let cut_index = match report.kind {
        crate::agent::LoopKind::ToolCall => crate::session::find_loop_cut_index(
            &entries,
            &report.tool_name,
            &report.canonical_args,
        ),
        crate::agent::LoopKind::TextEcho => {
            crate::session::find_text_loop_cut_index(&entries, &report.canonical_args)
        }
    };
    let mut removed = 0u32;
    if app.session.session_store.has_active_session() {
        match app
            .session
            .session_store
            .apply_loop_cut(cut_index, &report.tool_name, &report.args_preview, report.count)
        {
            Ok(n) => removed = n,
            Err(e) => return LoopRecoveryDecision::Failed(e.to_string()),
        }
        // The history was rewritten: reset the guard baseline so the next
        // outgoing request is not flagged as drift.
        app.session.reset_history_guard();
    }

    app.loop_recovery_count += 1;
    let prompt = match report.kind {
        crate::agent::LoopKind::ToolCall => crate::agent::loop_recovery_prompt(
            &report.tool_name,
            &report.args_preview,
            report.count,
            app.loop_recovery_count,
            max_recoveries,
        ),
        crate::agent::LoopKind::TextEcho => crate::agent::text_loop_recovery_prompt(
            &report.args_preview,
            report.count,
            app.loop_recovery_count,
            max_recoveries,
        ),
    };
    LoopRecoveryDecision::Recover { prompt, removed }
}

/// Apply a loop-recovery decision: show the system message and either
/// restart the agent with the recovery prompt or end the turn.
///
/// Shared by the `LoopDetected` event (tool-call loop) and the `Done(Ok)`
/// text-echo trip. Returns the restarted agent when recovering (the caller
/// swaps the channels), or `None` when the turn ends (budget exhausted or
/// cut failed). The caller must have already drained events, finalized the
/// turn, and zeroed `loop_free_tool_calls`.
fn apply_loop_recovery(
    app: &mut App,
    report: &crate::agent::LoopReport,
) -> Option<crate::turn::RestartedAgent> {
    match decide_loop_recovery(app, report) {
        LoopRecoveryDecision::Exhausted => {
            app.add_system_message(&format!(
                "⚠️ Loop recovery failed after {} attempts — the model keeps repeating the same action. Intervene manually.",
                app.config.config.loop_detection.max_recoveries,
            ));
            app.maybe_auto_scroll();
            None
        }
        LoopRecoveryDecision::Failed(e) => {
            app.add_system_message(&format!("Loop cut failed: {e}"));
            app.maybe_auto_scroll();
            None
        }
        LoopRecoveryDecision::Recover {
            prompt,
            removed,
        } => {
            app.add_system_message(&format!(
                "⚠️ Loop detected: {} ×{} — truncated {removed} messages, continuing. (recovery {}/{})",
                report.summary(),
                report.count,
                app.loop_recovery_count,
                app.config.config.loop_detection.max_recoveries,
            ));
            app.maybe_auto_scroll();
            Some(crate::turn::restart_agent(app, prompt))
        }
    }
}

/// The normalized visible text of the current turn's response.
///
/// `streaming_content` accumulates the turn's visible text tokens and is
/// cleared in `start_streaming` (every turn, including re-prompts), so at
/// `Done(Ok)` it is exactly this turn's text — empty on a reasoning-only turn
/// (which sends `Done` without a `SaveAssistantMessage`, so reading the
/// session store would return the *previous* turn's text and re-record a
/// stale key). For a text-only turn it equals the saved content block's text,
/// so the normalized key matches what the cut logic compares against.
fn last_assistant_text(app: &App) -> String {
    crate::agent::normalize_text(&app.agent.streaming_content)
}

/// Calculate the visible height of the chat area in lines.
/// Terminal height minus input area, status bar, and working indicator (if active).
#[must_use]
pub fn get_chat_visible_height(app: &App) -> usize {
    let input_height = crate::tui::layout::LayoutRenderer::input_area_height(app);
    let status_height = 1;
    app.ui
        .terminal_height
        .saturating_sub(input_height + status_height)
}
