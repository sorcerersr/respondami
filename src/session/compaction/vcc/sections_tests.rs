//! Tests for VCC section extractors.

use super::sections::{
    dedup_preferences_against_goals, extract_commits, extract_files, extract_goals,
    extract_outstanding_context, extract_path, extract_preferences, format_commits,
    format_file_activity,
};
use super::normalize::VccBlock;
use serde_json::json;

fn user(text: &str) -> VccBlock {
    VccBlock::User { text: text.to_string() }
}

fn tool_call(name: &str, path: Option<&str>) -> VccBlock {
    let args = match path {
        Some(p) => json!({ "path": p }),
        None => json!({}),
    };
    VccBlock::ToolCall { name: name.to_string(), args }
}

fn bash(command: &str, output: &str, exit: Option<i32>) -> VccBlock {
    VccBlock::Bash { command: command.to_string(), output: output.to_string(), exit_code: exit }
}

#[test]
fn extract_path_handles_keys() {
    assert_eq!(extract_path(&json!({ "path": "a.rs" })).as_deref(), Some("a.rs"));
    assert_eq!(extract_path(&json!({ "file_path": "b.rs" })).as_deref(), Some("b.rs"));
    assert_eq!(extract_path(&json!({ "filePath": "c.rs" })).as_deref(), Some("c.rs"));
    assert_eq!(extract_path(&json!({ "file": "d.rs" })).as_deref(), Some("d.rs"));
    assert_eq!(extract_path(&json!({})), None);
}

#[test]
fn extract_goals_first_user_block() {
    let blocks = vec![user("Please implement the login feature with tests")];
    let goals = extract_goals(&blocks);
    assert_ne!(goals, Vec::<String>::new());
    assert!(goals[0].contains("login"));
}

#[test]
fn extract_goals_drops_noise() {
    let blocks = vec![user("ok"), user("yes"), user("Please implement the login feature with tests")];
    let goals = extract_goals(&blocks);
    assert!(goals.iter().all(|g| !g.eq_ignore_ascii_case("ok")));
}

#[test]
fn extract_files_categorizes() {
    let blocks = vec![
        tool_call("read", Some("a.rs")),
        tool_call("edit", Some("b.rs")),
        tool_call("write", Some("c.rs")),
    ];
    let (read, modified, created) = extract_files(&blocks);
    assert!(read.contains("a.rs"));
    // `write` is in both write and create sets; the dedup drops it from created.
    assert!(modified.contains("b.rs"));
    assert!(modified.contains("c.rs"));
    assert!(!created.contains("c.rs"));
}

#[test]
fn format_file_activity_lines() {
    let blocks = vec![tool_call("read", Some("a.rs")), tool_call("edit", Some("b.rs"))];
    let lines = format_file_activity(&blocks);
    assert!(lines.iter().any(|l| l.starts_with("Modified:")));
    assert!(lines.iter().any(|l| l.starts_with("Read:")));
}

#[test]
fn extract_commits_from_git_commit() {
    let blocks = vec![bash("git commit -m \"fix bug\"", "abc1234 committed", Some(0))];
    let commits = extract_commits(&blocks);
    assert_eq!(commits.len(), 1);
    assert_eq!(commits[0].1, "fix bug");
}

#[test]
fn format_commits_with_hash() {
    let commits = vec![(Some("abc1234".to_string()), "fix bug".to_string())];
    let lines = format_commits(&commits, 8);
    assert!(lines[0].contains("abc1234"));
    assert!(lines[0].contains("fix bug"));
}

#[test]
fn extract_outstanding_context_blocker_lines() {
    let blocks = vec![
        VccBlock::Assistant { text: "The build is failing because of a missing dependency.".to_string() },
    ];
    let items = extract_outstanding_context(&blocks);
    assert_ne!(items, Vec::<String>::new());
}

#[test]
fn test_extract_preferences() {
    let blocks = vec![user("Please always use tabs for indentation")];
    let prefs = extract_preferences(&blocks);
    assert_ne!(prefs, Vec::<String>::new());
}

#[test]
fn test_dedup_preferences_against_goals() {
    let goals = vec!["Always use tabs".to_string()];
    let prefs = vec!["Always use tabs".to_string(), "Never push to main".to_string()];
    let deduped = dedup_preferences_against_goals(prefs, &goals);
    assert_eq!(deduped.len(), 1);
    assert!(deduped[0].contains("Never push"));
}
