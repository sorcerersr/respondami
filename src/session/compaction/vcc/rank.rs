//! VCC brief ranking — score blocks and select the highest-signal ones under a
//! size budget.
//!
//! `rank_brief_blocks` scores each block; `select_ranked_brief_blocks` keeps the
//! most recent blocks (for local continuity) plus the highest-scoring blocks
//! until the char budget is reached. Ported from pi-vcc's `rank.ts`.

use super::brief::compile_brief;
use super::normalize::VccBlock;
use super::sections::extract_path;

const DEFAULT_MAX_BLOCKS: usize = 80;
const DEFAULT_RECENT_BLOCKS: usize = 16;
const TRIVIAL_BASH_PENALTY: i64 = 16;

const EDIT_TOOLS: &[&str] = &["edit", "write", "multiedit", "quick_edit", "target_edit", "apply_patch"];
const READ_TOOLS: &[&str] = &["read", "glob", "grep", "ls", "find", "semantic_query", "semantic_grep", "semantic_show"];

fn matches_any(tools: &[&str], name: &str) -> bool {
    tools.iter().any(|t| t.eq_ignore_ascii_case(name))
}

fn is_test_command(cmd: &str) -> bool {
    let lower = cmd.to_lowercase();
    let has_runner = ["bun", "npm", "pnpm", "yarn", "node", "pytest", "cargo", "go", "mvn", "gradle"]
        .iter()
        .any(|r| lower.contains(r));
    let has_action = ["test", "spec", "check", "lint", "build", "tsc"]
        .iter()
        .any(|a| lower.contains(a));
    has_runner && has_action
}

fn is_workflow_command(cmd: &str) -> bool {
    let lower = cmd.to_lowercase();
    let git_actions = [
        "commit", "push", "merge", "rebase", "revert", "cherry-pick", "tag", "reset", "checkout",
        "branch",
    ];
    if lower.contains("git ") && git_actions.iter().any(|a| lower.contains(&format!("git {a}"))) {
        return true;
    }
    lower.contains("gh pr ") || lower.contains("gh issue ")
}

/// A bash command whose every meaningful line is pure scaffolding.
fn is_trivial_only_bash(raw: &str) -> bool {
    for line in raw.lines() {
        let t = line.trim();
        if t.is_empty() {
            continue;
        }
        let trivial = t.starts_with("set -")
            || t.starts_with("set +")
            || t.starts_with("cd ")
            || t == "cd"
            || t.starts_with("export ")
            || t.starts_with("source ")
            || t.starts_with(". ")
            || t == "pwd"
            || t == "true"
            || t == ":"
            || t.starts_with('#')
            || t.starts_with("ls")
            || t.starts_with("echo")
            || t == "clear"
            || t.starts_with("sleep");
        if !trivial {
            return false;
        }
    }
    true
}

fn bash_command_from_block(block: &VccBlock) -> Option<String> {
    match block {
        VccBlock::Bash { command, .. } => Some(command.clone()),
        VccBlock::ToolCall { name, args } => {
            if name.eq_ignore_ascii_case("bash") {
                args.get("command").and_then(serde_json::Value::as_str).map(str::to_string)
            } else {
                None
            }
        }
        _ => None,
    }
}

fn path_from_block(block: &VccBlock) -> Option<String> {
    match block {
        VccBlock::ToolCall { args, .. } => extract_path(args),
        VccBlock::Bash { command, .. } => {
            command.split_whitespace().find_map(|tok| {
                tok.contains('.')
                    .then(|| tok.to_string())
                    .filter(|t| t.chars().next().is_some_and(|c| c.is_ascii_alphanumeric() || c == '.' || c == '/'))
            })
        }
        _ => None,
    }
}

struct RankedBlock {
    index: usize,
    score: i64,
}

/// Score a single block.
fn score_block(
    block: &VccBlock,
    index: usize,
    total: usize,
    modified_files: &std::collections::HashSet<String>,
    read_files: &std::collections::HashSet<String>,
) -> i64 {
    let mut score = 0i64;
    let recency = if total <= 1 {
        0
    } else {
        ((index as f64 / (total - 1) as f64) * 12.0).round() as i64
    };
    score += recency;

    match block {
        VccBlock::User { .. } => score += 18,
        VccBlock::Assistant { .. } => score += 10,
        VccBlock::ToolResult { .. } => score += 1,
        VccBlock::Bash { command, exit_code, .. } => {
            score += 8;
            if exit_code.is_some_and(|c| c != 0) {
                score += 24;
            }
            if is_test_command(command) {
                score += 22;
            }
            if is_workflow_command(command) {
                score += 14;
            }
            if is_trivial_only_bash(command) && !exit_code.is_some_and(|c| c != 0) {
                score -= TRIVIAL_BASH_PENALTY;
            }
        }
        VccBlock::ToolCall { name, args } => {
            let command = bash_command_from_block(block);
            if matches_any(EDIT_TOOLS, name) {
                score += 34;
            } else if command.as_deref().is_some_and(is_test_command) {
                score += 26;
            } else if matches_any(READ_TOOLS, name) {
                score += 6;
            } else {
                score += 12;
            }
            if command.as_deref().is_some_and(is_workflow_command) {
                score += 14;
            }
            if command.as_deref().is_some_and(is_trivial_only_bash) {
                score -= TRIVIAL_BASH_PENALTY;
            }
            let _ = args;
        }
    }

    if let Some(path) = path_from_block(block) {
        if modified_files.contains(&path) {
            score += 18;
        }
        if read_files.contains(&path) {
            score += 6;
        }
    }

    if let VccBlock::ToolResult { text, .. } = block
        && text.len() > 1000
    {
        score -= 8;
    }

    score
}

/// A ranked selection of brief blocks.
#[must_use]
pub fn select_ranked_brief_blocks(blocks: &[VccBlock], max_brief_chars: Option<usize>) -> Vec<VccBlock> {
    let max_blocks = DEFAULT_MAX_BLOCKS;
    // Fast path: nothing to trim.
    if blocks.len() <= max_blocks && max_brief_chars.is_none() {
        return blocks.to_vec();
    }

    let modified_files: std::collections::HashSet<String> =
        blocks.iter().filter_map(path_from_block).collect();
    let read_files: std::collections::HashSet<String> = modified_files.clone();
    let total = blocks.len();
    let scored: Vec<RankedBlock> = blocks
        .iter()
        .enumerate()
        .map(|(i, b)| RankedBlock {
            index: i,
            score: score_block(b, i, total, &modified_files, &read_files),
        })
        .collect();

    let preserve_recent = std::cmp::min(DEFAULT_RECENT_BLOCKS, max_blocks);
    let mut selected: std::collections::HashSet<usize> = std::collections::HashSet::new();
    let mut used_chars = 0usize;

    // Keep the latest blocks for local continuity (newest first).
    let start = blocks.len().saturating_sub(preserve_recent);
    for i in (start..blocks.len()).rev() {
        if matches!(blocks[i], VccBlock::ToolResult { .. }) {
            continue;
        }
        if selected.contains(&i) {
            continue;
        }
        if let Some(budget) = max_brief_chars {
            let cost = compile_brief(&blocks[i..=i]).len() + 1;
            if used_chars + cost > budget {
                continue;
            }
            used_chars += cost;
        }
        selected.insert(i);
    }

    // Add highest-scoring blocks until the budget is reached.
    let mut ordered = scored.iter().collect::<Vec<_>>();
    ordered.sort_by(|a, b| b.score.cmp(&a.score).then(b.index.cmp(&a.index)));
    for item in ordered {
        if selected.len() >= max_blocks {
            break;
        }
        if selected.contains(&item.index) {
            continue;
        }
        if matches!(blocks[item.index], VccBlock::ToolResult { .. }) {
            continue;
        }
        if let Some(budget) = max_brief_chars {
            let cost = compile_brief(&blocks[item.index..=item.index]).len() + 1;
            if used_chars + cost > budget {
                continue;
            }
            used_chars += cost;
        }
        selected.insert(item.index);
    }

    let mut indices: Vec<usize> = selected.into_iter().collect();
    indices.sort_unstable();
    indices.into_iter().map(|i| blocks[i].clone()).collect()
}
