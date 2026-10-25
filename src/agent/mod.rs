//! Agent loop and tool orchestration.
//!
//! Spawns the agent task via `run_agent_with_snapshot()`, handles streaming
//! responses, tool execution, retry logic, and cooperative cancellation.
//!
//! Rust guideline compliant 2026-02-21

mod prompts;
mod streaming;
pub mod token_estimation;

#[cfg(test)]
mod mod_tests;
#[cfg(test)]
mod streaming_tests;
#[cfg(test)]
mod token_estimation_tests;

#[doc(inline)]
pub use prompts::get_system_prompt;
#[doc(inline)]
pub use streaming::{AgentResponse, build_provider, stream_response_from_messages};

use std::io;
use std::path::Path;
use std::sync::{Arc, Mutex, PoisonError};

use crate::config::Config;
use crate::history_guard::{HistoryGuard, MessageDigest, message_digest};
use crate::hooks::HookRegistry;
use crate::session::{
    AgentMessage, CompactionEngine, ContentBlock, RequestTokenUsage, SessionStore, ToolCall,
};
use crate::skills::Skill;
use crate::tools::rtk::rewrite_command;
use crate::tools::{CancelGuard, ToolRegistry};
use crate::tui::AbortReason;
use crate::tui::AgentEvent;
use crate::tui::CompactionReason;

use std::path::PathBuf;
use tokio::sync::mpsc;
use tokio::sync::watch;

/// Build the system prompt with AGENTS.md appended (if available) and skills XML block.
///
/// Structure:
/// 1. Base system prompt
/// 2. AGENTS.md wrapped in `<project_context>` XML block (if found)
/// 3. Skills XML block (if any)
/// 4. Current date and working directory
///
/// Returns `(system_prompt, load_error)`. On error, `system_prompt` is the
/// standard prompt (fallback).
#[must_use]
pub fn build_system_prompt_with_agents_md(
    cwd: &Path,
    skills: &[Skill],
) -> (String, Option<io::Error>) {
    let mut prompt = get_system_prompt().to_string();
    let mut load_error = None;

    // Append AGENTS.md wrapped in XML block
    match crate::agents_md::load_agents_md(cwd) {
        Ok(Some((content, path))) => {
            let path_str = path.display();
            prompt.push_str("\n\n<project_context>\n\n");
            prompt.push_str("Project-specific instructions and guidelines:\n\n");
            prompt.push_str(&format!(
                "<project_instructions path=\"{}\">\n{}\n</project_instructions>\n\n</project_context>",
                path_str,
                content.trim()
            ));
        }
        Ok(None) => {}
        Err(e) => {
            load_error = Some(e);
        }
    }

    // Append skills XML block (always, even on AGENTS.md error)
    let skills_xml = crate::skills::format_skills_for_prompt(skills);
    if !skills_xml.is_empty() {
        prompt.push_str(&skills_xml);
    }

    // Append date and working directory
    prompt.push_str(&build_date_and_cwd(cwd));

    (prompt, load_error)
}

/// Append "Current date: YYYY-MM-DD" and "Current working directory: /path".
fn build_date_and_cwd(cwd: &Path) -> String {
    let date = chrono::Local::now().format("%Y-%m-%d").to_string();
    let cwd_str = cwd.display().to_string();
    format!("\nCurrent date: {date}\nCurrent working directory: {cwd_str}")
}

/// Build context messages from the session, including system prompt.
///
/// Returns `(messages, load_error)`. On AGENTS.md load error, messages use
/// the standard system prompt and the error is returned separately.
///
/// Also stores the built prompt in the session's cache
/// (`set_system_prompt`) so the per-frame status-bar path never re-reads
/// AGENTS.md from disk (H2).
#[must_use]
pub fn build_context_with_system(
    session_store: &mut SessionStore,
    cwd: &Path,
    skills: &[Skill],
) -> (Vec<AgentMessage>, Option<io::Error>) {
    let (system_prompt, load_error) = build_system_prompt_with_agents_md(cwd, skills);
    session_store.set_system_prompt(system_prompt.clone());
    let mut messages = vec![AgentMessage::system(system_prompt)];
    messages.extend(session_store.build_context());
    (messages, load_error)
}

/// Run the agent loop with a snapshot of context messages and cooperative cancellation.
///
/// Does NOT take ownership of the `SessionStore`. The caller retains full access
/// to the conversation at all times, making cancellation safe.
///
/// All session bookkeeping is sent via `AgentEvent` (`SaveAssistantMessage`, `SaveToolResult`).
/// Tool call display is sent via `AgentEvent` (`ToolCallStart`, `ToolCallDone`).
/// Cancellation is cooperative via a watch channel — the task checks at natural break points.
#[expect(
    clippy::too_many_arguments,
    reason = "agent entry point takes the full turn context; a struct would hide the data flow"
)]
pub async fn run_agent_with_snapshot(
    context_messages: Vec<AgentMessage>,
    user_message: String,
    config: Config,
    cwd: PathBuf,
    tool_registry: ToolRegistry,
    mut hook_registry: HookRegistry,
    skills: Vec<crate::skills::Skill>,
    tx: mpsc::Sender<AgentEvent>,
    cancel_rx: watch::Receiver<bool>,
    rtk_state: crate::tools::rtk::RtkState,
    history_guard: Arc<Mutex<HistoryGuard>>,
) {
    let provider = match build_provider(&config) {
        Ok(p) => p,
        Err(e) => {
            let _ = tx
                .send(AgentEvent::Done(Err(AbortReason::Error(format!(
                    "Failed to build provider: {e}"
                )))))
                .await;
            return;
        }
    };

    let compaction_engine = CompactionEngine::from_config(&config);

    // Build initial context from snapshot + user message
    let mut current_context = context_messages;
    current_context.push(AgentMessage::user(user_message.clone()));

    // Per-message digests, kept in lockstep with `current_context` for the
    // history guard (vLLM prefix-cache stability).
    let mut context_digests: Vec<MessageDigest> =
        current_context.iter().map(message_digest).collect();

    let mut current_request_usage: RequestTokenUsage = RequestTokenUsage::default();
    // Retry tracking for transient provider errors
    let mut stream_retry_count = 0u32;
    let max_stream_retries = if config.retry.enabled {
        config.retry.max_retries
    } else {
        0
    };
    let base_delay_ms = config.retry.base_delay_ms;

    loop {
        // Check for cooperative cancellation at natural break point
        if *cancel_rx.borrow() {
            let _ = tx
                .send(AgentEvent::Done(Err(AbortReason::UserCancelled)))
                .await;
            return;
        }

        // Check if compaction is needed BEFORE streaming the next response.
        // Positioned here so that tool calls from the previous response have already
        // executed. On the first iteration, current_request_usage is (0, 0) so
        // should_compact() always returns false — no premature compaction.
        if compaction_engine.should_compact(current_request_usage.input_tokens) {
            let _ = tx
                .send(AgentEvent::NeedsCompaction {
                    total_tokens: current_request_usage.input_tokens,
                    reason: CompactionReason::Threshold,
                    retry_user_message: Some(
                        "Continue the current task from where you left off.".to_string(),
                    ),
                })
                .await;
            return;
        }

        // Stream response with retry on transient errors.
        // Modeled on pi-coding-agent: retry empty responses, network errors, rate limits,
        // and server errors with exponential backoff. Context overflow and billing errors
        // are NOT retried (compaction and user action respectively).
        // History guard: verify this outgoing context's prefix is byte-identical
        // to the previous outgoing context before the provider sees it.
        check_outgoing_history(&history_guard, &context_digests, &current_context);
        let response = loop {
            match stream_response_from_messages(
                &provider,
                &config,
                &current_context,
                &tx,
                &tool_registry,
                cancel_rx.clone(),
            )
            .await
            {
                Ok(resp) => {
                    let has_tool_calls = resp.has_tool_calls();
                    // Reject empty responses — retry if possible.
                    // A response with only reasoning/thinking (no visible content) is valid
                    // and should NOT be treated as empty. The model may think without speaking.
                    tracing::debug!(
                        text_len = resp.text().len(),
                        has_tool_calls = has_tool_calls,
                        has_thinking = resp.has_thinking(),
                        "Received agent response"
                    );
                    if resp.is_empty() {
                        // Log context on first empty response (before any retry)
                        if stream_retry_count == 0 {
                            log_context_before_retry(&current_context);
                        }
                        if !handle_retry(
                            &tx,
                            &cancel_rx,
                            &mut stream_retry_count,
                            max_stream_retries,
                            base_delay_ms,
                            "Provider returned empty response",
                            None,
                        )
                        .await
                        {
                            break AgentResponse {
                                content: Vec::new(),
                                usage: None,
                            };
                        }
                        // Remove the empty response from context before retry
                        if matches!(current_context.last(), Some(AgentMessage::Assistant { .. })) {
                            current_context.pop();
                            context_digests.pop();
                        }
                        continue;
                    }
                    // Success after retries
                    if stream_retry_count > 0 {
                        let _ = tx
                            .send(AgentEvent::RetryEnd {
                                success: true,
                                attempt: stream_retry_count,
                            })
                            .await;
                        stream_retry_count = 0;
                    }
                    break resp;
                }
                Err(e) => {
                    // Check if this was an abort (user cancellation or HTTP abort)
                    if let Some(abort_reason) = e.downcast_ref::<AbortReason>() {
                        match abort_reason {
                            AbortReason::UserCancelled | AbortReason::StreamAborted => {
                                let _ = tx.send(AgentEvent::Done(Err(abort_reason.clone()))).await;
                                return;
                            }
                            _ => {}
                        }
                    }

                    // Context overflow → compaction, not retry
                    if e.downcast_ref::<crate::provider::ProviderError>()
                        .is_some_and(|pe| {
                            matches!(pe, crate::provider::ProviderError::ContextOverflow { .. })
                        })
                    {
                        let _ = tx
                            .send(AgentEvent::NeedsCompaction {
                                total_tokens: current_request_usage.total(),
                                reason: CompactionReason::Overflow,
                                retry_user_message: Some(user_message.clone()),
                            })
                            .await;
                        return;
                    }

                    // Check if error is retryable (network, rate limit, server error, etc.)
                    let is_retryable = e
                        .downcast_ref::<crate::provider::ProviderError>()
                        .is_some_and(super::provider::ProviderError::is_retryable);

                    if is_retryable {
                        // Log context on first retryable error (before any retry)
                        if stream_retry_count == 0 {
                            log_context_before_retry(&current_context);
                        }
                        // Extract retry_after from HTTP errors
                        let retry_after = e
                            .downcast_ref::<crate::provider::ProviderError>()
                            .and_then(|pe| {
                                if let crate::provider::ProviderError::Http {
                                    retry_after, ..
                                } = pe
                                {
                                    *retry_after
                                } else {
                                    None
                                }
                            });

                        let error_msg = e.to_string();
                        if !handle_retry(
                            &tx,
                            &cancel_rx,
                            &mut stream_retry_count,
                            max_stream_retries,
                            base_delay_ms,
                            &error_msg,
                            retry_after,
                        )
                        .await
                        {
                            break AgentResponse {
                                content: Vec::new(),
                                usage: None,
                            };
                        }
                        continue;
                    }

                    // Non-retryable error — fail immediately
                    let _ = tx
                        .send(AgentEvent::Done(Err(AbortReason::Error(format!(
                            "Error: {e} — try sending your message again."
                        )))))
                        .await;
                    break AgentResponse {
                        content: Vec::new(),
                        usage: None,
                    };
                }
            }
        };

        // If the retry loop broke with an empty response, Done(Err) was already
        // sent by handle_retry() or the non-retryable error handler. Do not
        // process further to avoid sending a second Done(Ok) that overwrites it.
        if response.is_empty() {
            break;
        }

        // Reasoning-only response: model thought but produced no visible output.
        // Don't save empty assistant message to session (would corrupt context).
        // End the turn normally — reasoning was already displayed to the user.
        if response.text().trim().is_empty()
            && !response.has_tool_calls()
            && response.has_thinking()
        {
            if let Some(usage) = &response.usage {
                let _ = tx.send(AgentEvent::Usage(usage.clone())).await;
            }
            let _ = tx.send(AgentEvent::Done(Ok(()))).await;
            break;
        }

        let has_tool_calls = response.has_tool_calls();

        if let Some(usage) = &response.usage {
            // Accumulate using max+delta pattern
            let current = RequestTokenUsage {
                input_tokens: current_request_usage.input_tokens.max(usage.prompt_tokens),
                output_tokens: current_request_usage
                    .output_tokens
                    .max(usage.completion_tokens),
                estimated: false,
            };
            current_request_usage = current;
            let _ = tx.send(AgentEvent::Usage(usage.clone())).await;
        }

        if !has_tool_calls {
            // Final response (content, no tool calls): save to session before ending.
            let _ = tx
                .send(AgentEvent::SaveAssistantMessage {
                    content: response.content.clone(),
                    usage: response.usage.clone(),
                })
                .await;
            let _ = tx.send(AgentEvent::Done(Ok(()))).await;
            break;
        }

        // Execute tool calls. The per-call body lives in
        // `execute_tool_call` (M5); this loop only handles the
        // loop-level outcomes.
        let tool_calls: Vec<crate::session::ToolCall> =
            response.tool_calls().into_iter().cloned().collect();
        let mut tool_ctx = ToolCallCtx {
            tx: &tx,
            config: &config,
            cwd: &cwd,
            rtk_state: &rtk_state,
            tool_registry: &tool_registry,
            hook_registry: &mut hook_registry,
            cancel_rx: &cancel_rx,
            skills: &skills,
            response: &response,
            current_context: &mut current_context,
            context_digests: &mut context_digests,
            first_tool_call: true,
        };
        for tc in &tool_calls {
            match execute_tool_call(&mut tool_ctx, tc).await {
                ToolCallOutcome::Continue | ToolCallOutcome::Blocked => {}
                ToolCallOutcome::Cancelled => return,
            }
        }
    }
}

/// Per-call state for [`execute_tool_call`].
///
/// Bundles the captured state of `run_agent_with_snapshot` that the
/// per-tool-call body needs (M5).
struct ToolCallCtx<'a> {
    tx: &'a mpsc::Sender<AgentEvent>,
    config: &'a Config,
    cwd: &'a Path,
    rtk_state: &'a crate::tools::rtk::RtkState,
    tool_registry: &'a ToolRegistry,
    hook_registry: &'a mut HookRegistry,
    cancel_rx: &'a watch::Receiver<bool>,
    skills: &'a [crate::skills::Skill],
    response: &'a AgentResponse,
    current_context: &'a mut Vec<AgentMessage>,
    context_digests: &'a mut Vec<MessageDigest>,
    /// `true` before the first tool call of this response, so only the
    /// first call's `SaveAssistantMessage` carries the full content blocks
    /// and usage.
    first_tool_call: bool,
}

/// Outcome of [`execute_tool_call`]: keep the tool-call loop going, or end
/// the turn (user cancellation).
#[derive(Debug)]
enum ToolCallOutcome {
    /// The tool ran; keep looping.
    Continue,
    /// A `PreToolUse` hook blocked the tool; the blocked result was already
    /// saved, keep looping.
    Blocked,
    /// The user cancelled; the turn is already ended.
    Cancelled,
}

/// Execute one tool call: cancellation check, `activate_skill`
/// interception, `PreToolUse` hooks, registry execution, `PostToolUse` hooks,
/// and the canonical save + live-context push.
///
/// Extracted from `run_agent_with_snapshot` (M5): the per-call body is
/// self-contained; only the loop-level outcome is reported back.
async fn execute_tool_call(ctx: &mut ToolCallCtx<'_>, tc: &ToolCall) -> ToolCallOutcome {
    // Check for cancellation before each tool call
    if *ctx.cancel_rx.borrow() {
        let _ = ctx
            .tx
            .send(AgentEvent::Done(Err(AbortReason::UserCancelled)))
            .await;
        return ToolCallOutcome::Cancelled;
    }

    // Handle activate_skill tool calls directly.
    // These are intercepted before normal tool execution.
    // SkillActivation and SaveToolResult are sent to the TUI — no ToolCallStart/ToolCallDone.
    if tc.name == "activate_skill" {
        // Extract skill name from arguments
        let skill_name = tc
            .arguments
            .get("name")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        if skill_name.is_empty() {
            // Invalid call — no SkillActivation event (an empty name must not
            // pollute the TUI's active_skills set or the saved session header).
            // Return an error tool result so the model can correct itself.
            let error_result = "Error: skill name is required".to_string();
            push_persisted_context_pair(
                ctx.current_context,
                ctx.context_digests,
                tc,
                &error_result,
            );
            // Persist assistant+tool to session for correct alternation pattern.
            // Without this, the activate_skill tool call becomes orphaned
            // (no matching tool result) when context is rebuilt from JSONL.
            let _ = ctx
                .tx
                .send(AgentEvent::SaveAssistantMessage {
                    content: vec![ContentBlock::ToolCall {
                        tool_call: tc.clone(),
                    }],
                    usage: None,
                })
                .await;
            let _ = ctx
                .tx
                .send(AgentEvent::SaveToolResult {
                    tool_call_id: tc.id.clone(),
                    tool_name: tc.name.clone(),
                    tool_args: tc.arguments.clone(),
                    result: error_result,
                    images: Vec::new(),
                })
                .await;
            return ToolCallOutcome::Continue;
        }
        // Send SkillActivation event to the TUI (only visual feedback)
        let _ = ctx
            .tx
            .send(AgentEvent::SkillActivation {
                skill_name: skill_name.clone(),
            })
            .await;
        // Register the skill's frontmatter hooks into the local registry so
        // they fire for the rest of this turn (activation-scoped).
        if let Some(skill) = ctx.skills.iter().find(|s| s.name == skill_name) {
            crate::hooks::register_skill_hooks(ctx.hook_registry, skill);
        }
        // Build tool result: activation confirmation + full SKILL.md content
        let mut result = format!("Skill '{skill_name}' activated.");
        // Read and inject the SKILL.md body (frontmatter stripped, L3)
        if let Some(skill) = ctx.skills.iter().find(|s| s.name == skill_name) {
            match crate::skills::skill_body(&skill.file_path) {
                Some(body) => {
                    result.push_str(&format!(
                        "\n\n<skill_instructions name=\"{skill_name}\">\n{body}\n</skill_instructions>"
                    ));
                }
                None => {
                    result.push_str(&format!(
                        "\n\n[Error reading skill file '{}': unreadable]",
                        skill.file_path.display()
                    ));
                }
            }
        }
        push_persisted_context_pair(ctx.current_context, ctx.context_digests, tc, &result);
        // Persist assistant+tool to session for correct alternation pattern.
        // Without this, the activate_skill tool call becomes orphaned
        // (no matching tool result) when context is rebuilt from JSONL,
        // corrupting the message alternation pattern for the LLM.
        let _ = ctx
            .tx
            .send(AgentEvent::SaveAssistantMessage {
                content: vec![ContentBlock::ToolCall {
                    tool_call: tc.clone(),
                }],
                usage: None,
            })
            .await;
        let _ = ctx
            .tx
            .send(AgentEvent::SaveToolResult {
                tool_call_id: tc.id.clone(),
                tool_name: tc.name.clone(),
                tool_args: tc.arguments.clone(),
                result,
                images: Vec::new(),
            })
            .await;
        return ToolCallOutcome::Continue;
    }

    // Run PreToolUse hooks: a blocked hook (exit 2) stops the tool; its
    // stderr is injected as the model's feedback. Successful hook stdout is
    // a side effect and never reaches the model.
    let (tool_blocked, blocked_error, _) = crate::hooks::run_tool_hooks(
        ctx.tx,
        ctx.hook_registry,
        crate::hooks::HookEvent::PreToolUse,
        ctx.cwd,
        &tc.name,
        &tc.arguments,
        None,
    )
    .await;

    if tool_blocked {
        // Tool call blocked — send error as tool result
        let _ = ctx
            .tx
            .send(AgentEvent::ToolCallStart {
                tool_call_id: tc.id.clone(),
                tool_name: tc.name.clone(),
                tool_args: tc.arguments.clone(),
            })
            .await;
        let _ = ctx
            .tx
            .send(AgentEvent::ToolCallDone {
                tool_call_id: tc.id.clone(),
                result: format!("Tool call blocked by hook: {blocked_error}"),
                has_error: true,
                cancelled: false,
                rtk_original: None,
            })
            .await;
        let blocked_result = format!("Tool call blocked by hook: {blocked_error}");
        // Persist assistant message for this tool call to maintain alternation
        // in JSONL (assistant->tool->assistant->tool pattern).
        let _ = ctx
            .tx
            .send(AgentEvent::SaveAssistantMessage {
                content: vec![crate::session::ContentBlock::ToolCall {
                    tool_call: tc.clone(),
                }],
                usage: None,
            })
            .await;
        let _ = ctx
            .tx
            .send(AgentEvent::SaveToolResult {
                tool_call_id: tc.id.clone(),
                tool_name: tc.name.clone(),
                tool_args: tc.arguments.clone(),
                result: blocked_result.clone(),
                images: Vec::new(),
            })
            .await;
        // Add to context as a failed tool result
        push_persisted_context_pair(
            ctx.current_context,
            ctx.context_digests,
            tc,
            &blocked_result,
        );
        return ToolCallOutcome::Blocked;
    }

    // Signal TUI: tool call is starting (show pending message)
    let _ = ctx
        .tx
        .send(AgentEvent::ToolCallStart {
            tool_call_id: tc.id.clone(),
            tool_name: tc.name.clone(),
            tool_args: tc.arguments.clone(),
        })
        .await;

    // RTK rewrite: intercept bash tool calls
    let rtk_db_dir = ctx.cwd.join(".respondami").join("rtk");
    let (exec_name, exec_args, original_cmd) =
        if tc.name == "bash" && ctx.config.rtk.enabled && ctx.rtk_state.is_available() {
            if let Some(rtk_path) = &ctx.rtk_state.path {
                if let Some(cmd) = tc.arguments.get("command").and_then(|v| v.as_str()) {
                    if let Some(rewritten) = rewrite_command(rtk_path, &rtk_db_dir, cmd).await {
                        let mut new_args = tc.arguments.clone();
                        new_args["command"] = serde_json::Value::String(rewritten.clone());
                        (tc.name.clone(), new_args, Some(cmd.to_string()))
                    } else {
                        (tc.name.clone(), tc.arguments.clone(), None)
                    }
                } else {
                    (tc.name.clone(), tc.arguments.clone(), None)
                }
            } else {
                (tc.name.clone(), tc.arguments.clone(), None)
            }
        } else {
            (tc.name.clone(), tc.arguments.clone(), None)
        };

    // Create output channel for streaming tool output
    let (output_tx, mut output_rx) = tokio::sync::mpsc::channel::<String>(64);
    let main_tx = ctx.tx.clone();

    // Spawn a task to forward output chunks to the main channel
    let output_forwarder = tokio::spawn(async move {
        while let Some(chunk) = output_rx.recv().await {
            if !chunk.is_empty() {
                let _ = main_tx.send(AgentEvent::ToolOutput(chunk)).await;
            }
        }
    });

    // Create per-tool-call cancellation guard and bridge the user's
    // cancel signal into it. The watcher task sets the guard as soon
    // as the watch channel flips (Esc/Ctrl+C) or its sender is dropped
    // (TUI shutdown). Tools clean up from their own guard checks — the
    // agent loop never drops the tool future to escape.
    let tool_cancel = CancelGuard::new();
    let mut watcher_rx = ctx.cancel_rx.clone();
    let watcher_guard = tool_cancel.clone();
    let cancel_watcher = tokio::spawn(async move {
        let _ = watcher_rx.wait_for(|v| *v).await;
        watcher_guard.cancel();
    });

    let result = ctx
        .tool_registry
        .execute(
            &exec_name,
            exec_args.clone(),
            ctx.cwd,
            Some(&output_tx),
            &tool_cancel,
        )
        .await;
    cancel_watcher.abort(); // tool returned — no leak
    let has_error = result.is_err();
    // A tool error that coincides with a set guard is a user-initiated
    // cancellation, not a tool failure (distinguishes the TUI display).
    let cancelled = has_error && tool_cancel.is_cancelled();

    // Drop the sender to signal the forwarder to stop
    drop(output_tx);
    // Give forwarder a moment to flush, then abort if needed
    let _ = tokio::time::timeout(std::time::Duration::from_millis(50), output_forwarder).await;

    // When tool has an error, send the error message as the result content.
    // This gives the LLM feedback on why the tool failed (e.g. edit rejection reason).
    // Matches pi-coding-agent behavior: Anthropic SDK sends error message as tool content.
    let (final_result, tool_images) = match result {
        Ok(output) => (output.content, output.images),
        Err(e) => (e.to_string(), Vec::new()),
    };

    // Vision gate: drop image attachments when the model can't see them.
    let (final_result, tool_images) =
        apply_vision_gate(ctx.config.provider.vision, final_result, tool_images);

    // Run PostToolUse hooks (only for successful tool calls). These are
    // side effects; their stdout never reaches the model. A blocked hook
    // (exit 2) is an objection the model should still hear, so its stderr is
    // appended to the model-bound tool result (the blocked-hook exception).
    let mut blocked_posttool: Vec<String> = Vec::new();
    if !has_error {
        let (_, _, post_results) = crate::hooks::run_tool_hooks(
            ctx.tx,
            ctx.hook_registry,
            crate::hooks::HookEvent::PostToolUse,
            ctx.cwd,
            &tc.name,
            &tc.arguments,
            Some(&serde_json::json!(final_result.clone())),
        )
        .await;
        for (hook_name, hook_result) in post_results {
            if hook_result.blocked() {
                // The tool already ran; a blocked PostToolUse is an
                // objection the model should still hear.
                blocked_posttool.push(hook_result.stderr.clone());
            } else if !hook_result.success() {
                tracing::warn!(
                    hook = %hook_name,
                    exit_code = hook_result.exit_code,
                    stderr = %hook_result.stderr,
                    "PostToolUse hook failed"
                );
            }
        }
    }

    // Model-bound tool result: the clean tool output, plus any blocked
    // `PostToolUse` stderr (the blocked-hook exception). The TUI
    // (`ToolCallDone`) shows the clean `final_result`; the session store and
    // live context both receive `model_result`, so they stay byte-identical.
    let model_result = if blocked_posttool.is_empty() {
        final_result.clone()
    } else {
        let mut r = final_result.clone();
        for reason in &blocked_posttool {
            r.push_str(&format!("\n\n[hook blocked: {reason}]"));
        }
        r
    };

    // Signal TUI: tool call finished (update pending message with result)
    // Note: the tool result sent to TUI does NOT include hook instructions —
    // those are appended to the model-bound result only.
    let _ = ctx
        .tx
        .send(AgentEvent::ToolCallDone {
            tool_call_id: tc.id.clone(),
            result: final_result.clone(),
            has_error,
            cancelled,
            rtk_original: original_cmd.clone(),
        })
        .await;

    // Persist assistant message for this tool call to maintain alternation
    // in JSONL (assistant->tool->assistant->tool pattern).
    // First tool call carries the response's non-tool blocks (thinking,
    // text) plus this tool call; subsequent calls carry just their own.
    // The same blocks are saved to the session and pushed to the live
    // context so the two histories stay byte-identical (vLLM
    // prefix-cache stability).
    let assistant_blocks = canonical_turn_blocks(&ctx.response.content, tc, ctx.first_tool_call);
    let assistant_usage = if ctx.first_tool_call {
        ctx.response.usage.clone()
    } else {
        None
    };
    ctx.first_tool_call = false;
    let _ = ctx
        .tx
        .send(AgentEvent::SaveAssistantMessage {
            content: assistant_blocks.clone(),
            usage: assistant_usage.clone(),
        })
        .await;
    // Send tool result for TUI to save to session
    let _ = ctx
        .tx
        .send(AgentEvent::SaveToolResult {
            tool_call_id: tc.id.clone(),
            tool_name: tc.name.clone(),
            tool_args: tc.arguments.clone(),
            result: model_result.clone(),
            images: tool_images.clone(),
        })
        .await;

    // Add to context for next iteration — the same blocks and usage as
    // saved above, so live and persisted history never diverge.
    let assistant_msg = AgentMessage::assistant_with_blocks(assistant_blocks, assistant_usage);
    let assistant_digest = message_digest(&assistant_msg);
    let tool_msg = AgentMessage::tool(
        tc.id.clone(),
        tc.name.clone(),
        tc.arguments.clone(),
        model_result,
        tool_images,
    );
    let tool_digest = message_digest(&tool_msg);
    ctx.current_context.push(assistant_msg);
    ctx.context_digests.push(assistant_digest);
    ctx.current_context.push(tool_msg);
    ctx.context_digests.push(tool_digest);

    ToolCallOutcome::Continue
}

/// Push the assistant+tool message pair for an already-persisted tool call
/// (`activate_skill`, hook-blocked call) to the live context, keeping the
/// parallel digest list in lockstep.
fn push_persisted_context_pair(
    context: &mut Vec<AgentMessage>,
    digests: &mut Vec<MessageDigest>,
    tc: &ToolCall,
    result: &str,
) {
    let assistant = AgentMessage::assistant_with_blocks(
        vec![ContentBlock::ToolCall {
            tool_call: tc.clone(),
        }],
        None,
    );
    let tool = AgentMessage::tool(
        tc.id.clone(),
        tc.name.clone(),
        tc.arguments.clone(),
        result.to_string(),
        Vec::new(),
    );
    push_context_message(context, digests, assistant);
    push_context_message(context, digests, tool);
}

/// Push a message to the live context and its digest to the parallel digest
/// list, keeping the two in lockstep.
fn push_context_message(
    context: &mut Vec<AgentMessage>,
    digests: &mut Vec<MessageDigest>,
    message: AgentMessage,
) {
    let digest = message_digest(&message);
    digests.push(digest);
    context.push(message);
}

/// Verify the outgoing context prefix against the history guard before each
/// request.
///
/// The guard compares per-message digests with the previous outgoing context
/// and logs a structured warning (never blocking) when the prefix is no
/// longer byte-identical and append-only — the condition under which
/// provider prefix caching (e.g. vLLM) silently stops hitting.
fn check_outgoing_history(
    history_guard: &Mutex<HistoryGuard>,
    digests: &[MessageDigest],
    context: &[AgentMessage],
) {
    let mut guard = history_guard.lock().unwrap_or_else(PoisonError::into_inner);
    let Some(violation) = guard.check_and_update(digests) else {
        return;
    };
    let current_preview = context
        .get(violation.index)
        .map(format_message_preview)
        .unwrap_or_else(|| "  <context ended here>".to_string());
    tracing::warn!(
        index = violation.index,
        violation = %violation,
        current = %current_preview,
        "History prefix drift — provider prefix cache will miss from this point"
    );
}

/// Canonical assistant content blocks for one tool call of a multi-tool response.
///
/// First tool call: the response's non-tool blocks (thinking, text — in
/// stream order) plus this tool call. Later tool calls: just this tool call.
///
/// The same blocks are sent to the session (`SaveAssistantMessage`) and
/// pushed to the live context, so the two representations stay byte-identical
/// across turns. Without this, the live and persisted histories diverge at
/// every multi-tool-call position and provider prefix caching breaks. The
/// per-tool-call split (assistant→tool→assistant→tool) is preserved for the
/// JSONL alternation requirement.
///
/// The thinking (reasoning) blocks are intentionally kept: they are reserialized
/// on every subsequent request. This grows within-turn token usage but keeps
/// the history byte-stable — the deliberate trade documented in the
/// cache-stable history design (see `docs/plans/2026-09-12-cache-stable-history-design.md`).
#[must_use]
fn canonical_turn_blocks(
    content: &[ContentBlock],
    tool_call: &ToolCall,
    first: bool,
) -> Vec<ContentBlock> {
    let mut blocks: Vec<ContentBlock> = if first {
        content
            .iter()
            .filter(|block| !matches!(block, ContentBlock::ToolCall { .. }))
            .cloned()
            .collect()
    } else {
        Vec::new()
    };
    blocks.push(ContentBlock::ToolCall {
        tool_call: tool_call.clone(),
    });
    blocks
}

/// Vision gate: when the model doesn't support vision, replace image
/// attachments with a text note so the model is never sent images it can't
/// see. Gated here (not in the read tool) so the tool stays config-agnostic.
/// The note replaces the content so the model knows the read succeeded but the
/// image was dropped. When vision is enabled or there are no images, the pair
/// is returned unchanged.
pub(crate) fn apply_vision_gate(
    vision: bool,
    content: String,
    images: Vec<crate::image::ImageContent>,
) -> (String, Vec<crate::image::ImageContent>) {
    if !vision && !images.is_empty() {
        (
            format!(
                "[Vision disabled: read {} image(s) but the model cannot see them. \
                 Set `provider.vision: true` in config to enable image support.]",
                images.len()
            ),
            Vec::new(),
        )
    } else {
        (content, images)
    }
}

/// Log the last 3 context messages before a retry attempt.
///
/// Fires once per empty-response event (when `stream_retry_count` == 0),
/// enabling users to spot corrupted tool results at INFO log level.
fn log_context_before_retry(context: &[AgentMessage]) {
    let total = context.len();
    let tail: Vec<&AgentMessage> = context.iter().rev().take(3).collect();
    let lines: Vec<String> = tail.into_iter().rev().map(format_message_preview).collect();
    tracing::info!(
        messages = total,
        "Context before retry:\n{}",
        lines.join("\n")
    );
}

/// Format a single `AgentMessage` as a one-line preview for diagnostics logging.
fn format_message_preview(msg: &AgentMessage) -> String {
    match msg {
        AgentMessage::System { content } => {
            let preview = truncate_content(content, 500);
            format!("  [system] — {preview}")
        }
        AgentMessage::User { content } => {
            let preview = truncate_content(content, 500);
            format!("  [user] — {preview}")
        }
        AgentMessage::Assistant {
            content: blocks, ..
        } => {
            let tool_names: Vec<&str> = blocks
                .iter()
                .filter_map(|b| {
                    if let crate::session::ContentBlock::ToolCall { tool_call } = b {
                        Some(tool_call.name.as_str())
                    } else {
                        None
                    }
                })
                .collect();
            let prefix = if tool_names.is_empty() {
                "[assistant]".to_string()
            } else {
                format!("[assistant: {}]", tool_names.join(", "))
            };
            let text_preview = truncate_content(&msg.text(), 500);
            format!("  {prefix} — {text_preview}")
        }
        AgentMessage::Tool {
            tool_name, result, ..
        } => {
            let preview = truncate_content(result, 500);
            format!("  [tool: {tool_name}] — {preview}")
        }
    }
}

/// Truncate content to max chars, escape newlines, append "…" if truncated.
fn truncate_content(content: &str, max_chars: usize) -> String {
    let escaped = content.replace("\r\n", "\\n").replace('\n', "\\n");
    if escaped.chars().count() <= max_chars {
        escaped
    } else {
        let truncated: String = escaped.chars().take(max_chars).collect();
        format!("{truncated}…")
    }
}

/// Handle a retry attempt with exponential backoff.
///
/// Returns `true` if the caller should retry (continue the loop).
/// Returns `false` if retries are exhausted (caller should break/fail).
///
/// Modeled on pi-coding-agent's `_prepareRetry`:
/// - Increments retry counter
/// - Emits `RetryStart` event with delay info
/// - Sleeps with exponential backoff (abortable via `cancel_rx`)
/// - Emits `RetryEnd` on exhaustion
///
/// When `retry_after` is `Some`, uses the server-provided delay instead of
/// exponential backoff (matches fantasy's Retry-After header support).
async fn handle_retry(
    tx: &mpsc::Sender<AgentEvent>,
    cancel_rx: &watch::Receiver<bool>,
    retry_count: &mut u32,
    max_retries: u32,
    base_delay_ms: u64,
    error_msg: &str,
    retry_after: Option<std::time::Duration>,
) -> bool {
    *retry_count += 1;
    if *retry_count > max_retries {
        // Exhausted — emit final failure event
        let _ = tx
            .send(AgentEvent::RetryEnd {
                success: false,
                attempt: *retry_count - 1,
            })
            .await;
        let _ = tx
            .send(AgentEvent::Done(Err(AbortReason::Error(format!(
                "Provider error after {} retries: {}",
                *retry_count, error_msg
            )))))
            .await;
        return false;
    }

    // Compute delay: use Retry-After header if present, otherwise exponential backoff with jitter.
    let delay = if let Some(d) = retry_after {
        d
    } else {
        // Clamp the shift to 32: `1u64 << n` panics in debug for n >= 64, and a
        // misconfigured max_retries must not crash the agent loop. The delay is
        // already impractically large well before the clamp kicks in.
        let shift = (*retry_count - 1).min(32);
        let base = base_delay_ms * (1u64 << shift);
        // Add ±25% random jitter to prevent thundering herd
        let jitter = 0.75 + fastrand::f64() * 0.5; // 0.75..1.25
        std::time::Duration::from_millis((base as f64 * jitter) as u64)
    };

    let delay_ms = delay.as_millis() as u64;
    let _ = tx
        .send(AgentEvent::RetryStart {
            attempt: *retry_count,
            max_attempts: max_retries + 1,
            delay_ms,
            error: error_msg.to_string(),
        })
        .await;
    tokio::time::sleep(delay).await;
    // Check cancellation during delay
    if *cancel_rx.borrow() {
        let _ = tx
            .send(AgentEvent::Done(Err(AbortReason::UserCancelled)))
            .await;
        // Return false to break out, but cancellation already sent Done(Err)
        return false;
    }
    true
}
