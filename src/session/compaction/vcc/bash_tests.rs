//! Tests for bash command compression.

use super::bash::compress_bash;

#[test]
fn compress_simple_command() {
    assert_eq!(compress_bash("ls -la"), "ls -la");
}

#[test]
fn compress_strips_cd_prefix() {
    let out = compress_bash("cd /home/user && cargo build");
    assert_eq!(out, "cargo build");
}

#[test]
fn compress_strips_pipe_tail() {
    let out = compress_bash("cat file.txt | head -20");
    assert_eq!(out, "cat file.txt");
}

#[test]
fn compress_drops_trivial_preamble() {
    let out = compress_bash("set -e\ncd /tmp\necho hello");
    assert_eq!(out, "echo hello");
}

#[test]
fn compress_keeps_only_line_when_trivial() {
    let out = compress_bash("cd /tmp");
    assert_eq!(out, "cd /tmp");
}

#[test]
fn compress_caps_length() {
    let long = format!("{} {}", "echo", "x".repeat(500));
    let out = compress_bash(&long);
    assert!(out.len() <= 240);
    assert!(out.ends_with("..."));
}

#[test]
fn compress_heredoc_drops_body() {
    let cmd = "cat > file.txt <<EOF\nline1\nline2\nline3\nEOF";
    let out = compress_bash(cmd);
    // File-writer heredoc: body dropped, opener kept.
    assert!(out.contains("cat > file.txt"));
    assert!(!out.contains("line2"));
}

#[test]
fn compress_joins_multiple_lines() {
    let out = compress_bash("echo a\necho b");
    assert_eq!(out, "echo a; echo b");
}
