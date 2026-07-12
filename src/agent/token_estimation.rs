//! Fallback token estimation for display when provider Usage events haven't arrived yet.
//!
//! Uses a simple len/4 heuristic (matching Crush's `approxTokenCount`) to estimate
//! token counts from message text. This is good enough for a smooth percentage display
//! during streaming, avoiding the "?" gap.

use crate::session::AgentMessage;

/// Approximate token count from a string using the len/4 heuristic.
///
/// This is a rough estimate: most tokenizers produce ~4 characters per token
/// for English text. The formula `(len + 3) / 4` rounds up.
#[must_use]
pub fn approx_token_count(s: &str) -> u32 {
    if s.is_empty() {
        return 0;
    }
    s.len().div_ceil(4) as u32
}

/// Estimate input tokens from a list of messages sent to the model.
///
/// Walks all messages and sums up estimated tokens from their content,
/// role labels, tool call metadata, etc.
#[must_use]
pub fn estimate_tokens_from_messages(messages: &[AgentMessage]) -> u32 {
    let mut tokens = 0u32;
    for msg in messages {
        tokens += estimate_message_tokens(msg);
    }
    tokens
}

/// Estimate output tokens from accumulated streaming text.
///
/// Used during streaming to estimate how many output tokens have been generated
/// so far, based on the text accumulated in `streaming_content`.
#[must_use]
pub fn estimate_tokens_from_streamed_text(text: &str) -> u32 {
    approx_token_count(text)
}

/// Estimate tokens for a single `AgentMessage`.
fn estimate_message_tokens(msg: &AgentMessage) -> u32 {
    match msg {
        AgentMessage::System { content } => {
            approx_token_count("system") + approx_token_count(content)
        }
        AgentMessage::User { content } => {
            approx_token_count("user") + approx_token_count(content)
        }
        AgentMessage::Assistant { content: blocks, .. } => {
            let mut tokens = approx_token_count("assistant");
            for block in blocks {
                match block {
                    crate::session::ContentBlock::Text { text } => {
                        tokens += approx_token_count(text);
                    }
                    crate::session::ContentBlock::Thinking { thinking } => {
                        tokens += approx_token_count(thinking);
                    }
                    crate::session::ContentBlock::ToolCall { tool_call } => {
                        tokens += estimate_tool_call_tokens(tool_call);
                    }
                }
            }
            tokens
        }
        AgentMessage::Tool {
            tool_call_id,
            tool_name,
            tool_arguments,
            result,
        } => {
            approx_token_count("tool")
                + approx_token_count(tool_call_id)
                + approx_token_count(tool_name)
                + approx_token_count(&tool_arguments.to_string())
                + approx_token_count(result)
        }
    }
}

/// Estimate tokens for a tool call.
fn estimate_tool_call_tokens(tc: &crate::session::ToolCall) -> u32 {
    approx_token_count(&tc.id)
        + approx_token_count(&tc.name)
        + approx_token_count(&tc.arguments.to_string())
}
