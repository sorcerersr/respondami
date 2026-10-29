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
                let engine = CompactionEngine::from_config(&app.config.config);
                let provider = match build_provider(&app.config.config) {
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
                let config = app.config.config.clone();
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
            Ok(Some(AgentEvent::Done(Ok(())))) => {
                // Drain any remaining Token/Reasoning/Usage events from the channel.
                // Done may arrive while tokens are still buffered (channel cap 256).
                app.drain_pending_events(&mut rx);
                if let Some(snap) = app.agent.tracker.finalize_turn() {
                    app.session.session_store.save_token_rate(snap);
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
