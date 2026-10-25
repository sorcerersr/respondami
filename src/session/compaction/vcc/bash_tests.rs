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

#[test]
fn compress_cap_boundary_mid_cjk_char() {
    // 235 ASCII bytes + one 3-byte CJK char: the BASH_CAP-3 cut (byte 237)
    // lands mid-character. Panics before the floor_char_boundary fix.
    let long = format!("{} 你好", "x".repeat(235));
    let out = compress_bash(&long);
    assert!(out.len() <= 240);
    assert!(out.ends_with("..."));
    assert!(!out.contains('你'));
}

#[test]
fn compress_heredoc_preview_boundary_mid_cjk_char() {
    // 77 ASCII bytes + one 3-byte CJK char in the preview: the
    // HEREDOC_BODY_CAP-1 cut (byte 79) lands mid-character.
    let body = format!("{} 你好", "a".repeat(77));
    let cmd = format!("python3 <<EOF\n{body}\nEOF");
    let out = compress_bash(&cmd);
    assert!(out.contains("python3 <<EOF"));
    assert!(out.ends_with('…'));
    assert!(!out.contains('你'));
}
