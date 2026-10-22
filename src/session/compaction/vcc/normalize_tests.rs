//! Tests for VCC normalization.

use super::normalize::{
    clip, clip_sentence, collapse_skill_text, non_empty_lines, normalize_entries, parse_exit_code,
    sanitize, VccBlock,
};
use crate::session::{AgentMessage, SessionEntry, ToolCall};

fn user_entry(text: &str) -> SessionEntry {
    SessionEntry::new_message(None, AgentMessage::user(text.to_string()))
}

fn assistant_entry(text: &str) -> SessionEntry {
    SessionEntry::new_message(None, AgentMessage::assistant(text.to_string(), String::new(), vec![], None))
}

fn bash_call_entry(id: &str, command: &str) -> SessionEntry {
    let tc = ToolCall {
        id: id.to_string(),
        name: "bash".to_string(),
        arguments: serde_json::json!({ "command": command }),
    };
    SessionEntry::new_message(None, AgentMessage::assistant(String::new(), String::new(), vec![tc], None))
}

fn bash_result_entry(id: &str, result: &str) -> SessionEntry {
    SessionEntry::new_message(
        None,
        AgentMessage::tool(id.to_string(), "bash".to_string(), serde_json::json!({}), result.to_string(), vec![]),
    )
}

#[test]
fn sanitize_strips_ansi_and_control() {
    let input = "hello\x1b[31mred\x1b[0m world\r\nline2";
    let out = sanitize(input);
    assert!(!out.contains('\x1b'));
    assert!(out.contains("red"));
    assert!(out.contains("line2"));
    assert!(!out.contains('\r'));
}

#[test]
fn clip_prefers_word_boundary() {
    let text = "aaaa bbbb cccc dddd";
    let out = clip(text, 10);
    assert!(out.len() <= 10);
    assert!(!out.ends_with(' '));
}

#[test]
fn clip_short_text_unchanged() {
    assert_eq!(clip("hi", 10), "hi");
}

#[test]
fn clip_sentence_prefers_sentence_end() {
    let text = "First sentence. Second sentence here.";
    let out = clip_sentence(text, 20);
    assert!(out.ends_with('.'));
}

#[test]
fn non_empty_lines_trims_and_filters() {
    let lines = non_empty_lines("  a  \n\n  b  \n");
    assert_eq!(lines, vec!["a", "b"]);
}

#[test]
fn collapse_skill_text_replaces_block() {
    let text = "before <skill_instructions name=\"foo\">body</skill_instructions> after";
    let out = collapse_skill_text(text);
    assert!(out.contains("[skill: foo]"));
    assert!(!out.contains("body"));
}

#[test]
fn parse_exit_code_trailing() {
    assert_eq!(parse_exit_code("output\nExit code: 0"), Some(0));
    assert_eq!(parse_exit_code("out\nExit code: 2"), Some(2));
    assert_eq!(parse_exit_code("Command timed out after 30 seconds"), None);
    assert_eq!(parse_exit_code("no code"), None);
}

#[test]
fn normalize_user_and_assistant() {
    let entries = vec![user_entry("hello"), assistant_entry("world")];
    let blocks = normalize_entries(&entries);
    assert_eq!(blocks.len(), 2);
    assert!(matches!(&blocks[0], VccBlock::User { .. }));
    assert!(matches!(&blocks[1], VccBlock::Assistant { .. }));
}

#[test]
fn normalize_bash_pairs_call_and_result() {
    let entries = vec![
        bash_call_entry("t1", "ls -la"),
        bash_result_entry("t1", "file1\nExit code: 0"),
    ];
    let blocks = normalize_entries(&entries);
    assert_eq!(blocks.len(), 1);
    match &blocks[0] {
        VccBlock::Bash { command, output, exit_code } => {
            assert_eq!(command, "ls -la");
            assert!(output.contains("file1"));
            assert_eq!(*exit_code, Some(0));
        }
        _ => panic!("expected Bash block"),
    }
}

#[test]
fn normalize_unpaired_bash_emitted_empty() {
    let entries = vec![bash_call_entry("t1", "echo hi")];
    let blocks = normalize_entries(&entries);
    assert_eq!(blocks.len(), 1);
    match &blocks[0] {
        VccBlock::Bash { command, output, exit_code } => {
            assert_eq!(command, "echo hi");
            assert_eq!(output, "");
            assert_eq!(*exit_code, None);
        }
        _ => panic!("expected Bash block"),
    }
}

#[test]
fn normalize_unpaired_bash_keeps_call_position() {
    // A cancelled bash call (no result) must stay at the call position,
    // not be appended at the end of the block list.
    let entries = vec![
        user_entry("run it"),
        bash_call_entry("t1", "echo hi"),
        assistant_entry("done"),
    ];
    let blocks = normalize_entries(&entries);
    assert_eq!(blocks.len(), 3);
    match &blocks[1] {
        VccBlock::Bash { command, output, exit_code } => {
            assert_eq!(command, "echo hi");
            assert_eq!(output, "");
            assert_eq!(*exit_code, None);
        }
        _ => panic!("expected Bash block at call position"),
    }
}

#[test]
fn normalize_bash_result_fills_placeholder_in_place() {
    let entries = vec![
        user_entry("run it"),
        bash_call_entry("t1", "ls"),
        bash_result_entry("t1", "file1\nExit code: 0"),
        assistant_entry("done"),
    ];
    let blocks = normalize_entries(&entries);
    assert_eq!(blocks.len(), 3);
    match &blocks[1] {
        VccBlock::Bash { command, output, exit_code } => {
            assert_eq!(command, "ls");
            assert!(output.contains("file1"));
            assert_eq!(*exit_code, Some(0));
        }
        _ => panic!("expected Bash block at call position"),
    }
}

#[test]
fn normalize_skips_non_message_entries() {
    let session = SessionEntry::new_session("s".into(), "t".into(), "/".into(), "m".into(), 4096);
    let entries = vec![session, user_entry("hi")];
    let blocks = normalize_entries(&entries);
    assert_eq!(blocks.len(), 1);
}

#[test]
fn block_kind_strings() {
    assert_eq!(VccBlock::User { text: "x".into() }.kind(), "user");
    assert_eq!(VccBlock::Bash { command: "c".into(), output: "o".into(), exit_code: None }.kind(), "bash");
}
