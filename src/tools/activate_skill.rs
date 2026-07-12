//! Skill activation tools.
//!
//! These tools are intercepted by the agent loop and never reach this handler.
//! They exist in the registry only so the model knows they are available.

use super::{ToolDefinition, ToolHandler};
use async_trait::async_trait;
use std::path::Path;
use tokio::sync::mpsc;

/// Activate a skill by name.
///
/// This tool is intercepted by the agent loop and never actually executed.
/// It exists only in the tool registry so the model knows it can activate skills.
#[derive(Debug)]
pub struct ActivateSkillTool;

#[async_trait]
impl ToolHandler for ActivateSkillTool {
    async fn execute(
        &self,
        _args: serde_json::Value,
        _cwd: &Path,
        _output_tx: Option<&mpsc::Sender<String>>,
        _cancel: &super::CancelGuard,
    ) -> anyhow::Result<String> {
        // Should never be reached — intercepted by agent loop
        Ok("Skill activated".to_string())
    }
}

impl ToolDefinition for ActivateSkillTool {
    fn definition(&self) -> (String, String, serde_json::Value) {
        (
            "activate_skill".to_string(),
            "Activate a skill by name. Required before following any skill's instructions. \
            Available skills are listed in your skills section. Once activated, the skill \
            remains active for the rest of the session."
                .to_string(),
            serde_json::json!({
                "type": "object",
                "properties": {
                    "name": {
                        "type": "string",
                        "description": "The name of the skill to activate (e.g., \"refine\", \"planning-with-files\", \"pdf\")"
                    }
                },
                "required": ["name"],
                "additionalProperties": false
            }),
        )
    }
}
