//! Tests for VCC brief ranking.

use super::rank::select_ranked_brief_blocks;
use super::normalize::VccBlock;

fn user(text: &str) -> VccBlock {
    VccBlock::User { text: text.to_string() }
}

fn assistant(text: &str) -> VccBlock {
    VccBlock::Assistant { text: text.to_string() }
}

fn bash(command: &str, output: &str, exit: Option<i32>) -> VccBlock {
    VccBlock::Bash { command: command.to_string(), output: output.to_string(), exit_code: exit }
}

#[test]
fn select_fast_path_no_budget() {
    let blocks = vec![user("a"), assistant("b")];
    let selected = select_ranked_brief_blocks(&blocks, None);
    assert_eq!(selected.len(), 2);
}

#[test]
fn select_preserves_recent_blocks() {
    let blocks: Vec<VccBlock> = (0..50).map(|i| user(&format!("turn {i}"))).collect();
    let selected = select_ranked_brief_blocks(&blocks, None);
    // The most recent blocks should be present.
    assert!(selected.iter().any(|b| matches!(b, VccBlock::User { text } if text == "turn 49")));
}

#[test]
fn select_respects_block_cap() {
    let blocks: Vec<VccBlock> = (0..200).map(|i| user(&format!("turn {i}"))).collect();
    let selected = select_ranked_brief_blocks(&blocks, None);
    assert!(selected.len() <= 80);
}

#[test]
fn select_failing_bash_scores_high() {
    let blocks = vec![
        user("run tests"),
        bash("cargo test", "FAILED", Some(1)),
        assistant("let me fix it"),
    ];
    let selected = select_ranked_brief_blocks(&blocks, None);
    // The failing bash should be retained (high score).
    assert!(selected.iter().any(|b| matches!(b, VccBlock::Bash { exit_code: Some(1), .. })));
}
