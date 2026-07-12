//! Agent event types and partial tool call structures.
//!
//! Defines `AgentEvent` (messages from agent task to TUI loop), `PartialToolCall`
//! (accumulated during streaming), `CompactionReason`, and `AbortReason`.

use crate::session::{ContentBlock, Usage};

/// A partial tool call being accumulated during streaming.
#[derive(Debug, Clone)]
pub struct PartialToolCall {
    pub id: String,
    pub name: String,
    pub arguments: String,
}

/// Reason why compaction is needed.
#[derive(Debug, Clone, Copy)]
pub enum CompactionReason {
    /// Context usage exceeded threshold after a turn.
    Threshold,
    /// Server returned context window exceeded error.
    Overflow,
}

/// Messages sent from the agent task to the main TUI loop.
#[derive(Debug)]
pub enum AgentEvent {
    /// A new token arrived from the LLM.
    Token(String),
    /// The model started sending reasoning/thinking content.
    ThinkingStart,
    /// The model finished sending reasoning/thinking content.
    ThinkingEnd,
    /// Reasoning text delta — hidden by default, counted for t/s.
    Reasoning(String),
    /// A tool call is about to be executed; show a pending message.
    ToolCallStart { tool_call_id: String, tool_name: String, tool_args: serde_json::Value },
    /// Incremental output from a running tool call; append to pending message.
    ToolOutput(String),
    /// A tool call finished; update the pending message with results.
    ToolCallDone { tool_call_id: String, result: String, has_error: bool, rtk_original: Option<String> },
    /// Token usage from an LLM response.
    Usage(Usage),
    /// Compaction completed (display message).
    Compaction { tokens_saved: u32, message_count: u32 },
    /// The agent finished (or errored).
    /// `Err(AbortReason)` indicates the run was cancelled by the user.
    Done(Result<(), AbortReason>),
    /// Save an assistant message to the session.
    SaveAssistantMessage { content: Vec<ContentBlock>, usage: Option<Usage> },
    /// Save a tool result to the session.
    SaveToolResult {
        tool_call_id: String,
        tool_name: String,
        tool_args: serde_json::Value,
        result: String,
    },
    /// Agent signals that compaction is needed.
    /// Main thread performs compaction and restarts the agent.
    NeedsCompaction {
        /// Total context tokens at time of signal.
        total_tokens: u32,
        /// Why compaction is needed.
        reason: CompactionReason,
        /// User message to retry after compaction (only for Overflow).
        retry_user_message: Option<String>,
    },
    /// Progress update during compaction LLM call.
    CompactionProgress { message: String },
    /// Compaction completed with result.
    CompactionDone {
        tokens_before: u32,
        tokens_after: u32,
        messages_removed: u32,
    },
    /// Compaction failed.
    CompactionError { message: String },
    /// Auto-retry begins after transient provider error.
    RetryStart { attempt: u32, max_attempts: u32, delay_ms: u64, error: String },
    /// Auto-retry completes (success or final failure).
    RetryEnd { success: bool, attempt: u32 },
    /// A hook has executed; display a hook message.
    HookMessage { event: crate::hooks::HookEvent, hook_name: String, success: bool, stdout: String, stderr: String, tool_name: Option<String> },
    /// Skill activation — display a skill activation message and update `active_skills`.
    SkillActivation { skill_name: String },
}

/// Reason the agent run was aborted.
#[derive(Debug, Clone)]
pub enum AbortReason {
    /// User pressed ESC/Ctrl+C.
    UserCancelled,
    /// The streaming request was aborted (HTTP-level abort).
    StreamAborted,
    /// An actual error occurred (with message).
    Error(String),
}

impl std::fmt::Display for AbortReason {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AbortReason::UserCancelled => write!(f, "Operation cancelled by user"),
            AbortReason::StreamAborted => write!(f, "Streaming aborted"),
            AbortReason::Error(msg) => write!(f, "{msg}"),
        }
    }
}

impl std::error::Error for AbortReason {}
