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
mod token_estimation_tests;

#[doc(inline)]
pub use prompts::get_system_prompt;
#[doc(inline)]
pub use streaming::{AgentResponse, build_provider, stream_response_from_messages};

use std::io;
use std::path::Path;

use crate::config::Config;
use crate::hooks::{HookContext, HookRegistry, execute_hook};
use crate::session::{AgentMessage, CompactionEngine, RequestTokenUsage, SessionStore};
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
#[must_use]
pub fn build_context_with_system(
    session_store: &SessionStore,
    cwd: &Path,
    skills: &[Skill],
) -> (Vec<AgentMessage>, Option<io::Error>) {
    let (system_prompt, load_error) = build_system_prompt_with_agents_md(cwd, skills);
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
    reason = "render signature unified across all message types"
)]
pub async fn run_agent_with_snapshot(
    context_messages: Vec<AgentMessage>,
    user_message: String,
    config: Config,
    cwd: PathBuf,
    tool_registry: ToolRegistry,
    hook_registry: HookRegistry,
    mut active_skills: Vec<String>,
    skills: Vec<crate::skills::Skill>,
    tx: mpsc::Sender<AgentEvent>,
    cancel_rx: watch::Receiver<bool>,
    rtk_state: crate::tools::rtk::RtkState,
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

        // Execute tool calls
        let tool_calls: Vec<crate::session::ToolCall> =
            response.tool_calls().into_iter().cloned().collect();
        let rtk_db_dir = cwd.join(".respondami").join("rtk");
        // Track which tool call is first (gets full content in JSONL)
        let mut first_tool_call = true;

        for tc in &tool_calls {
            // Check for cancellation before each tool call
            if *cancel_rx.borrow() {
                let _ = tx
                    .send(AgentEvent::Done(Err(AbortReason::UserCancelled)))
                    .await;
                return;
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
                    // Invalid call — send error as SkillActivation with a message
                    let _ = tx
                        .send(AgentEvent::SkillActivation {
                            skill_name: "unknown".to_string(),
                        })
                        .await;
                    let error_result = "Error: skill name is required".to_string();
                    current_context.push(AgentMessage::assistant_with_blocks(
                        vec![crate::session::ContentBlock::ToolCall {
                            tool_call: tc.clone(),
                        }],
                        None,
                    ));
                    current_context.push(AgentMessage::tool(
                        tc.id.clone(),
                        tc.name.clone(),
                        tc.arguments.clone(),
                        error_result.clone(),
                    ));
                    // Persist assistant+tool to session for correct alternation pattern.
                    // Without this, the activate_skill tool call becomes orphaned
                    // (no matching tool result) when context is rebuilt from JSONL.
                    let _ = tx
                        .send(AgentEvent::SaveAssistantMessage {
                            content: vec![crate::session::ContentBlock::ToolCall {
                                tool_call: tc.clone(),
                            }],
                            usage: None,
                        })
                        .await;
                    let _ = tx
                        .send(AgentEvent::SaveToolResult {
                            tool_call_id: tc.id.clone(),
                            tool_name: tc.name.clone(),
                            tool_args: tc.arguments.clone(),
                            result: error_result,
                        })
                        .await;
                    continue;
                }
                // Send SkillActivation event to the TUI (only visual feedback)
                let _ = tx
                    .send(AgentEvent::SkillActivation {
                        skill_name: skill_name.clone(),
                    })
                    .await;
                // Update local active_skills snapshot so tool hooks fire in the same turn
                active_skills.push(skill_name.clone());
                // Build tool result: activation confirmation + full SKILL.md content
                let mut result = format!("Skill '{skill_name}' activated.");
                // Read and inject the SKILL.md content
                if let Some(skill) = skills.iter().find(|s| s.name == skill_name) {
                    match std::fs::read_to_string(&skill.file_path) {
                        Ok(content) => {
                            let content = content.splitn(2, "---").nth(2).unwrap_or(&content);
                            result.push_str(&format!(
                                "\n\n<skill_instructions name=\"{}\">\n{}\n</skill_instructions>",
                                skill_name,
                                content.trim()
                            ));
                        }
                        Err(e) => {
                            result.push_str(&format!(
                                "\n\n[Error reading skill file '{}': {}]",
                                skill.file_path.display(),
                                e
                            ));
                        }
                    }
                }
                current_context.push(AgentMessage::assistant_with_blocks(
                    vec![crate::session::ContentBlock::ToolCall {
                        tool_call: tc.clone(),
                    }],
                    None,
                ));
                current_context.push(AgentMessage::tool(
                    tc.id.clone(),
                    tc.name.clone(),
                    tc.arguments.clone(),
                    result.clone(),
                ));
                // Persist assistant+tool to session for correct alternation pattern.
                // Without this, the activate_skill tool call becomes orphaned
                // (no matching tool result) when context is rebuilt from JSONL,
                // corrupting the message alternation pattern for the LLM.
                let _ = tx
                    .send(AgentEvent::SaveAssistantMessage {
                        content: vec![crate::session::ContentBlock::ToolCall {
                            tool_call: tc.clone(),
                        }],
                        usage: None,
                    })
                    .await;
                let _ = tx
                    .send(AgentEvent::SaveToolResult {
                        tool_call_id: tc.id.clone(),
                        tool_name: tc.name.clone(),
                        tool_args: tc.arguments.clone(),
                        result,
                    })
                    .await;
                continue;
            }

            // Run PreToolUse hooks
            let pre_tool_hooks = hook_registry.hooks(crate::hooks::HookEvent::PreToolUse);
            let mut tool_blocked = false;
            let mut blocked_error = String::new();
            for hook in pre_tool_hooks {
                let context = HookContext {
                    event: crate::hooks::HookEvent::PreToolUse,
                    hook_name: hook.name.clone(),
                    cwd: cwd.clone(),
                    tool_name: Some(tc.name.clone()),
                    tool_input: Some(tc.arguments.clone()),
                    tool_result: None,
                    prompt: None,
                };
                let result = execute_hook(hook, &context).await;
                if result.blocked() {
                    tool_blocked = true;
                    blocked_error = result.stderr.clone();
                    // Send hook message to TUI
                    let _ = tx
                        .send(AgentEvent::HookMessage {
                            event: crate::hooks::HookEvent::PreToolUse,
                            hook_name: hook.name.clone(),
                            success: false,
                            stdout: result.stdout,
                            stderr: result.stderr,
                            tool_name: Some(tc.name.clone()),
                        })
                        .await;
                    break;
                } else if result.success() {
                    // Send hook message to TUI
                    let _ = tx
                        .send(AgentEvent::HookMessage {
                            event: crate::hooks::HookEvent::PreToolUse,
                            hook_name: hook.name.clone(),
                            success: true,
                            stdout: result.stdout,
                            stderr: String::new(),
                            tool_name: Some(tc.name.clone()),
                        })
                        .await;
                }
            }

            if tool_blocked {
                // Tool call blocked — send error as tool result
                let _ = tx
                    .send(AgentEvent::ToolCallStart {
                        tool_call_id: tc.id.clone(),
                        tool_name: tc.name.clone(),
                        tool_args: tc.arguments.clone(),
                    })
                    .await;
                let _ = tx
                    .send(AgentEvent::ToolCallDone {
                        tool_call_id: tc.id.clone(),
                        result: format!("Tool call blocked by hook: {blocked_error}"),
                        has_error: true,
                        rtk_original: None,
                    })
                    .await;
                let blocked_result = format!("Tool call blocked by hook: {blocked_error}");
                // Persist assistant message for this tool call to maintain alternation
                // in JSONL (assistant->tool->assistant->tool pattern).
                let _ = tx
                    .send(AgentEvent::SaveAssistantMessage {
                        content: vec![crate::session::ContentBlock::ToolCall {
                            tool_call: tc.clone(),
                        }],
                        usage: None,
                    })
                    .await;
                let _ = tx
                    .send(AgentEvent::SaveToolResult {
                        tool_call_id: tc.id.clone(),
                        tool_name: tc.name.clone(),
                        tool_args: tc.arguments.clone(),
                        result: blocked_result.clone(),
                    })
                    .await;
                // Add to context as a failed tool result
                current_context.push(AgentMessage::assistant_with_blocks(
                    vec![crate::session::ContentBlock::ToolCall {
                        tool_call: tc.clone(),
                    }],
                    None,
                ));
                current_context.push(AgentMessage::tool(
                    tc.id.clone(),
                    tc.name.clone(),
                    tc.arguments.clone(),
                    blocked_result,
                ));
                continue;
            }

            // Signal TUI: tool call is starting (show pending message)
            let _ = tx
                .send(AgentEvent::ToolCallStart {
                    tool_call_id: tc.id.clone(),
                    tool_name: tc.name.clone(),
                    tool_args: tc.arguments.clone(),
                })
                .await;

            // RTK rewrite: intercept bash tool calls
            let (exec_name, exec_args, original_cmd) = if tc.name == "bash"
                && config.rtk.enabled
                && rtk_state.is_available()
            {
                if let Some(rtk_path) = &rtk_state.path {
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
            let main_tx = tx.clone();

            // Spawn a task to forward output chunks to the main channel
            let output_forwarder = tokio::spawn(async move {
                while let Some(chunk) = output_rx.recv().await {
                    if !chunk.is_empty() {
                        let _ = main_tx.send(AgentEvent::ToolOutput(chunk)).await;
                    }
                }
            });

            // Create per-tool-call cancellation guard.
            let tool_cancel = CancelGuard::new();

            let result = tool_registry
                .execute(
                    &exec_name,
                    exec_args.clone(),
                    &cwd,
                    Some(&output_tx),
                    &tool_cancel,
                )
                .await;
            let has_error = result.is_err();

            // Drop the sender to signal the forwarder to stop
            drop(output_tx);
            // Give forwarder a moment to flush, then abort if needed
            let _ =
                tokio::time::timeout(std::time::Duration::from_millis(50), output_forwarder).await;

            // When tool has an error, send the error message as the result content.
            // This gives the LLM feedback on why the tool failed (e.g. edit rejection reason).
            // Matches pi-coding-agent behavior: Anthropic SDK sends error message as tool content.
            let final_result = match result {
                Ok(content) => content,
                Err(e) => e.to_string(),
            };

            // Run PostToolUse hooks (only for successful tool calls).
            // Collect hook output as instructions for the LLM (injected as synthetic tool call).
            let mut hook_instructions = String::new();
            if !has_error {
                let post_tool_hooks = hook_registry.hooks(crate::hooks::HookEvent::PostToolUse);
                for hook in post_tool_hooks {
                    let context = HookContext {
                        event: crate::hooks::HookEvent::PostToolUse,
                        hook_name: hook.name.clone(),
                        cwd: cwd.clone(),
                        tool_name: Some(tc.name.clone()),
                        tool_input: Some(tc.arguments.clone()),
                        tool_result: Some(serde_json::json!(final_result)),
                        prompt: None,
                    };
                    let hook_result = execute_hook(hook, &context).await;
                    // Send hook message to TUI
                    let _ = tx
                        .send(AgentEvent::HookMessage {
                            event: crate::hooks::HookEvent::PostToolUse,
                            hook_name: hook.name.clone(),
                            success: hook_result.success(),
                            stdout: hook_result.stdout.clone(),
                            stderr: hook_result.stderr.clone(),
                            tool_name: Some(tc.name.clone()),
                        })
                        .await;
                    if hook_result.blocked() {
                        hook_instructions.push_str(&format!(
                            "\n\n[PostToolUse hook '{}' blocked: {}]",
                            hook.name, hook_result.stderr
                        ));
                    } else if hook_result.success() && !hook_result.stdout.is_empty() {
                        hook_instructions.push_str(&format!(
                            "\n\n[PostToolUse hook '{}': {}]",
                            hook.name, hook_result.stdout
                        ));
                    }
                }
            }

            // Signal TUI: tool call finished (update pending message with result)
            // Note: the tool result sent to TUI does NOT include hook instructions —
            // those are injected as a separate synthetic tool call for the LLM only.
            let _ = tx
                .send(AgentEvent::ToolCallDone {
                    tool_call_id: tc.id.clone(),
                    result: final_result.clone(),
                    has_error,
                    rtk_original: original_cmd.clone(),
                })
                .await;

            // Persist assistant message for this tool call to maintain alternation
            // in JSONL (assistant->tool->assistant->tool pattern).
            // First tool call gets full content blocks; subsequent ones get just the tool call.
            // Without this per-call save, multi-tool-call responses produce
            // consecutive tool->tool messages in JSONL, corrupting context rebuild.
            let assistant_blocks = if first_tool_call {
                // Include all content blocks (text, thinking) plus this tool call
                let mut blocks = response.content.clone();
                blocks.push(crate::session::ContentBlock::ToolCall {
                    tool_call: tc.clone(),
                });
                blocks
            } else {
                vec![crate::session::ContentBlock::ToolCall {
                    tool_call: tc.clone(),
                }]
            };
            let assistant_usage = if first_tool_call {
                response.usage.clone()
            } else {
                None
            };
            first_tool_call = false;
            let _ = tx
                .send(AgentEvent::SaveAssistantMessage {
                    content: assistant_blocks,
                    usage: assistant_usage,
                })
                .await;
            // Send tool result for TUI to save to session
            let _ = tx
                .send(AgentEvent::SaveToolResult {
                    tool_call_id: tc.id.clone(),
                    tool_name: tc.name.clone(),
                    tool_args: tc.arguments.clone(),
                    result: final_result.clone(),
                })
                .await;

            // Add to context for next iteration
            current_context.push(AgentMessage::assistant_with_blocks(
                vec![crate::session::ContentBlock::ToolCall {
                    tool_call: tc.clone(),
                }],
                None,
            ));
            current_context.push(AgentMessage::tool(
                tc.id.clone(),
                tc.name.clone(),
                tc.arguments.clone(),
                final_result,
            ));

            // If there are hook instructions, inject them as a synthetic hook_instruction tool call/result.
            // This is visible only to the LLM — no TUI events, not saved to session.
            if !hook_instructions.is_empty() {
                let hook_tool_call = crate::session::ToolCall {
                    id: "hook_instruction".to_string(),
                    name: crate::tools::HOOK_INSTRUCTION_TOOL.to_string(),
                    arguments: serde_json::json!({}),
                };
                // Inject into agent context ONLY — no TUI events, no session save
                current_context.push(AgentMessage::assistant_with_blocks(
                    vec![crate::session::ContentBlock::ToolCall {
                        tool_call: hook_tool_call.clone(),
                    }],
                    None,
                ));
                current_context.push(AgentMessage::tool(
                    "hook_instruction".to_string(),
                    crate::tools::HOOK_INSTRUCTION_TOOL.to_string(),
                    serde_json::json!({}),
                    hook_instructions.trim().to_string(),
                ));
            }
        }
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
        let base = base_delay_ms * (1u64 << (*retry_count - 1));
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
