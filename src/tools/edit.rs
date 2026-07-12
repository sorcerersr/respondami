//! Edit tool — precise text replacement in files.
//!
//! Matches `oldText` exactly (or via fuzzy matching) and replaces with `newText`.
//! Handles UTF-8 BOM stripping/restoration and line ending normalization to preserve
//! original file encoding. Uses per-file serialization queues to prevent concurrent
//! edits on the same file.

use super::edit_diff::{
    apply_edits, detect_line_ending, normalize_to_lf, restore_line_endings, strip_bom,
};
use super::{CancelGuard, ToolDefinition, ToolHandler};
use async_trait::async_trait;
use std::path::{Path, PathBuf};
use tokio::sync::mpsc;

pub struct EditTool;

impl std::fmt::Debug for EditTool {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("EditTool")
    }
}

impl ToolDefinition for EditTool {
    fn definition(&self) -> (String, String, serde_json::Value) {
        (
            "edit".to_string(),
            "Edit a single file using exact text replacement. Every edits[].oldText must match a unique, non-overlapping region of the original file. If two changes affect the same block or nearby lines, merge them into one edit instead of emitting overlapping edits. Do not include large unchanged regions just to connect distant changes.".to_string(),
            serde_json::json!({
                "type": "object",
                "properties": {
                    "path": {"type": "string", "description": "Path to the file to edit (relative or absolute)"},
                    "edits": {
                        "type": "array",
                        "items": {
                            "type": "object",
                            "properties": {
                                "oldText": {"type": "string", "description": "Exact text for one targeted replacement. It must be unique in the original file and must not overlap with any other edits[].oldText in the same call."},
                                "newText": {"type": "string", "description": "Replacement text for this targeted edit."}
                            },
                            "required": ["oldText", "newText"],
                            "additionalProperties": false
                        },
                        "description": "One or more targeted replacements. Each edit is matched against the original file, not incrementally. Do not include overlapping or nested edits. If two changes touch the same block or nearby lines, merge them into one edit instead."
                    }
                },
                "required": ["path", "edits"],
                "additionalProperties": false
            }),
        )
    }
}

/// Prepared edit arguments after normalization.
struct PreparedArgs {
    path: String,
    edits: Vec<(String, String)>,
}

/// Preprocess edit arguments to handle model output quirks.
///
/// - JSON-stringified edits: some models send edits as a JSON string instead of an array
/// - Legacy flat params: some models use oldText/newText at top level instead of edits[]
fn prepare_args(args: &serde_json::Value) -> Result<PreparedArgs, String> {
    let path = args
        .get("path")
        .and_then(|v| v.as_str())
        .ok_or("Missing required parameter: path".to_string())?
        .to_string();

    // Handle edits field
    let edits_value = args.get("edits");

    // Some models send edits as a JSON string instead of an array
    let edits_array = match edits_value {
        Some(serde_json::Value::String(s)) => {
            // Try to parse the string as JSON array
            serde_json::from_str::<serde_json::Value>(s)
                .ok()
                .and_then(|v| v.as_array().map(Vec::to_owned))
        }
        Some(serde_json::Value::Array(a)) => Some(a.clone()),
        _ => None,
    };

    let mut edits: Vec<(String, String)> = edits_array
        .map(|arr| {
            arr.iter()
                .filter_map(|edit| {
                    let old = edit.get("oldText").and_then(|v| v.as_str())?;
                    let new = edit.get("newText").and_then(|v| v.as_str())?;
                    Some((old.to_string(), new.to_string()))
                })
                .collect()
        })
        .unwrap_or_default();

    // Legacy flat params: oldText/newText at top level
    if let (Some(old), Some(new)) = (
        args.get("oldText").and_then(|v| v.as_str()),
        args.get("newText").and_then(|v| v.as_str()),
    ) {
        edits.push((old.to_string(), new.to_string()));
    }

    if edits.is_empty() {
        return Err("Edit tool input is invalid. edits must contain at least one replacement."
            .to_string());
    }

    Ok(PreparedArgs { path, edits })
}

#[async_trait]
impl ToolHandler for EditTool {
    async fn execute(
        &self,
        args: serde_json::Value,
        cwd: &Path,
        _output_tx: Option<&mpsc::Sender<String>>,
        cancel: &CancelGuard,
    ) -> anyhow::Result<String> {
        // Prepare and validate arguments
        let prepared = prepare_args(&args)
            .map_err(|e| anyhow::anyhow!(e))?;

        let path = if Path::new(&prepared.path).is_absolute() {
            PathBuf::from(&prepared.path)
        } else {
            cwd.join(&prepared.path)
        };

        // Async existence check
        if tokio::fs::metadata(&path).await.is_err() {
            return Err(anyhow::anyhow!(
                "Could not edit file: {}. File does not exist.",
                prepared.path
            ));
        }
        if cancel.is_cancelled() {
            return Err(anyhow::anyhow!("Operation cancelled"));
        }

        // Read the file (async)
        let raw_content = tokio::fs::read_to_string(&path)
            .await
            .map_err(|e| anyhow::anyhow!("Failed to read file: {e}"))?;
        if cancel.is_cancelled() {
            return Err(anyhow::anyhow!("Operation cancelled"));
        }

        // Strip BOM before matching. The model will not include an invisible BOM in oldText.
        let (had_bom, content_no_bom) = strip_bom(&raw_content);

        // Detect and normalize line endings
        let original_ending = detect_line_ending(content_no_bom);
        let normalized_content = normalize_to_lf(content_no_bom);

        // Apply edits (exact match first, fuzzy fallback)
        let apply_result = apply_edits(&normalized_content, &prepared.edits, &prepared.path)
            .map_err(|e| anyhow::anyhow!(e))?;

        // Restore line endings
        let restored = restore_line_endings(&apply_result.new_content, original_ending);

        // Restore BOM if present
        let final_content = if had_bom {
            format!("\u{FEFF}{restored}")
        } else {
            restored
        };

        // Write the file (with per-file serialization, async)
        super::file_queue::with_file_queue(&path, async {
            tokio::fs::write(&path, &final_content).await
        })
        .await??;

        // Generate unified diff
        let diff = super::edit_diff::generate_diff(
            &apply_result.base_content,
            &apply_result.new_content,
            &prepared.path,
        );

        Ok(format!(
            "Successfully replaced {} block(s) in {}.\n\n{}",
            prepared.edits.len(),
            prepared.path,
            diff
        ))
    }
}
