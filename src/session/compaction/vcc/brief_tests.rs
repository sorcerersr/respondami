//! Tests for VCC brief rendering.

use super::brief::{cap_brief, cap_brief_default, compile_brief};
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
fn compile_brief_renders_user_and_assistant() {
    let blocks = vec![user("do the thing"), assistant("done")];
    let brief = compile_brief(&blocks);
    assert!(brief.contains("[user]"));
    assert!(brief.contains("do the thing"));
    assert!(brief.contains("[assistant]"));
    assert!(brief.contains("done"));
}

#[test]
fn compile_brief_renders_bash() {
    let blocks = vec![bash("ls -la", "file1", Some(0))];
    let brief = compile_brief(&blocks);
    assert!(brief.contains("ls -la"));
}

#[test]
fn compile_brief_empty() {
    assert_eq!(compile_brief(&[]), "");
}

#[test]
fn cap_brief_keeps_tail() {
    let text = (0..200).map(|i| format!("line {i}")).collect::<Vec<_>>().join("\n");
    let capped = cap_brief(&text, 10);
    let lines: Vec<&str> = capped.lines().collect();
    assert!(lines.len() <= 12); // 10 kept + omitted marker + blank
    assert!(capped.contains("line 199"));
}

#[test]
fn cap_brief_short_unchanged() {
    let text = "a\nb\nc";
    assert_eq!(cap_brief(text, 10), text);
}

#[test]
fn cap_brief_default_uses_max_lines() {
    let text = (0..200).map(|i| format!("line {i}")).collect::<Vec<_>>().join("\n");
    let capped = cap_brief_default(&text);
    assert!(capped.contains("line 199"));
}
