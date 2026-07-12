//! Tool registry and execution framework.
//!
//! Provides `ToolRegistry` for registering and executing tools, `CancelGuard`
//! for cooperative cancellation, and the `ToolHandler` trait for tool implementations.
//!
//! Rust guideline compliant 2026-02-21

use std::collections::HashMap;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc;

/// Lightweight cancellation guard for tool execution.
///
/// Clone is cheap (Arc clone). `is_cancelled()` is lock-free.
/// `cancel()` sets the flag — all clones see it immediately.
#[derive(Debug, Clone, Default)]
pub struct CancelGuard {
    inner: Arc<AtomicBool>,
}

impl CancelGuard {
    #[must_use]
    pub fn new() -> Self {
        Self {
            inner: Arc::new(AtomicBool::new(false)),
        }
    }

    /// Check if cancellation has been requested.
    /// Lock-free — safe to call from any context.
    #[must_use]
    pub fn is_cancelled(&self) -> bool {
        self.inner.load(Ordering::Relaxed)
    }

    /// Request cancellation. All clones of this guard see the change.
    pub fn cancel(&self) {
        self.inner.store(true, Ordering::Relaxed);
    }
}

mod read;
mod write;
mod edit;
mod bash;
mod edit_diff;
mod file_queue;
mod activate_skill;
pub mod rtk;

pub(crate) use bash::truncate_to_bytes;

#[cfg(test)]
mod edit_diff_tests;

#[cfg(test)]
mod read_tests;

#[cfg(test)]
mod write_tests;

#[cfg(test)]
mod activate_skill_tests;
#[cfg(test)]
mod bash_tests;
#[cfg(test)]
mod file_queue_tests;
#[cfg(test)]
mod rtk_tests;

/// Result of executing a tool.
///
/// Tools return `anyhow::Result<String>` directly. This struct is kept
/// for the `AgentEvent::SaveToolResult` session persistence, which needs
/// to store both content and error as a single unit.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolResult {
    pub content: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// Definition of a tool.
#[derive(Debug, Clone)]
pub struct ToolDef {
    pub name: String,
    pub description: String,
    pub schema: serde_json::Value,
}

/// Trait for tool handlers.
#[async_trait]
pub trait ToolHandler: Send + Sync {
    /// Execute the tool with arguments.
    ///
    /// `output_tx` is an optional channel for streaming incremental output.
    /// Tools that support streaming (e.g. bash) send chunks via this channel.
    /// Tools that don't support streaming ignore it.
    /// `cancel` is a cancellation guard — check `cancel.is_cancelled()` after every
    /// `await` point and return early if cancelled.
    async fn execute(
        &self,
        args: serde_json::Value,
        cwd: &Path,
        output_tx: Option<&mpsc::Sender<String>>,
        cancel: &CancelGuard,
    ) -> anyhow::Result<String>;
}

/// Name of the internal `hook_instruction` tool — never callable by the LLM.
/// Used to inject `PostToolUse` hook output as a synthetic tool call/result.
pub const HOOK_INSTRUCTION_TOOL: &str = "hook_instruction";

/// Registry of available tools.
///
/// `Arc` is used instead of `Box` so that `ToolRegistry` can be cloned cheaply.
/// Tool handlers are stateless singletons, so `Arc` is the right choice.
#[derive(Clone)]
pub struct ToolRegistry {
    pub tools: HashMap<String, Arc<dyn ToolHandler>>,
    pub definitions: Vec<ToolDef>,
}

impl std::fmt::Debug for ToolRegistry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ToolRegistry")
            .field("tools", &self.tools.keys().collect::<Vec<_>>())
            .field("definitions", &self.definitions)
            .finish()
    }
}

impl Default for ToolRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl ToolRegistry {
    #[must_use]
    pub fn new() -> Self {
        let mut registry = Self {
            tools: HashMap::new(),
            definitions: Vec::new(),
        };
        registry.register(read::ReadTool);
        registry.register(write::WriteTool);
        registry.register(edit::EditTool);
        registry.register(bash::BashTool);
        registry.register(activate_skill::ActivateSkillTool);
        // Register the internal hook_instruction tool definition (not callable by LLM).
        // It is used to inject PostToolUse hook output as a synthetic tool call/result.
        registry.definitions.push(ToolDef {
            name: HOOK_INSTRUCTION_TOOL.to_string(),
            description: "Internal tool for receiving instructions from hooks. You will never call this tool yourself.".to_string(),
            schema: serde_json::json!({
                "type": "object",
                "properties": {},
            }),
        });
        registry
    }

    fn register<H: ToolHandler + ToolDefinition + 'static>(&mut self, handler: H) {
        let (name, description, schema) = handler.definition();
        self.tools.insert(name.clone(), Arc::new(handler));
        self.definitions.push(ToolDef { name, description, schema });
    }

    /// Get tool info for building the LLM request.
    #[must_use]
    pub fn get_definitions(&self) -> &[ToolDef] {
        &self.definitions
    }

    /// Execute a tool by name.
    ///
    /// `output_tx` is an optional channel for streaming incremental output.
    /// `cancel` is a cancellation guard — passed to the tool handler.
    ///
    /// # Errors
    ///
    /// - Returns an error if no tool with the given name is registered.
    /// - Propagates errors from the tool handler (I/O, timeout, etc.).
    pub async fn execute(
        &self,
        name: &str,
        args: serde_json::Value,
        cwd: &Path,
        output_tx: Option<&mpsc::Sender<String>>,
        cancel: &CancelGuard,
    ) -> anyhow::Result<String> {
        match self.tools.get(name) {
            Some(handler) => handler.execute(args, cwd, output_tx, cancel).await,
            None => Err(anyhow::anyhow!("Unknown tool: {name}")),
        }
    }
}

/// Trait for tools to provide their definition.
pub trait ToolDefinition {
    fn definition(&self) -> (String, String, serde_json::Value);
}
