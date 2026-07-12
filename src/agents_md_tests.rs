use std::fs;
use std::io;
use tempfile::TempDir;

use crate::agents_md::{load_agents_md, GENERATE_CHAT_MESSAGE, GENERATE_PROMPT};

#[test]
fn load_agents_md_not_found() {
    let dir = TempDir::new().unwrap();
    let result = load_agents_md(dir.path());
    assert!(result.is_ok());
    assert_eq!(result.unwrap(), None);
}

#[test]
fn load_agents_md_root() {
    let dir = TempDir::new().unwrap();
    fs::write(dir.path().join("AGENTS.md"), "# Project Rules\nBe nice.").unwrap();
    let (content, path) = load_agents_md(dir.path()).unwrap().unwrap();
    assert!(content.contains("Project Rules"));
    assert!(content.contains("Be nice."));
    assert!(path.to_string_lossy().contains("AGENTS.md"));
}

#[test]
fn load_agents_md_hidden_fallback() {
    let dir = TempDir::new().unwrap();
    let hidden = dir.path().join(".respondami");
    fs::create_dir_all(&hidden).unwrap();
    fs::write(hidden.join("AGENTS.md"), "# Hidden Rules").unwrap();
    let (content, path) = load_agents_md(dir.path()).unwrap().unwrap();
    assert!(content.contains("Hidden Rules"));
    assert!(path.to_string_lossy().contains(".respondami"));
}

#[test]
fn load_agents_md_root_priority() {
    let dir = TempDir::new().unwrap();
    fs::write(dir.path().join("AGENTS.md"), "# Root Rules").unwrap();
    let hidden = dir.path().join(".respondami");
    fs::create_dir_all(&hidden).unwrap();
    fs::write(hidden.join("AGENTS.md"), "# Hidden Rules").unwrap();
    let (content, path) = load_agents_md(dir.path()).unwrap().unwrap();
    assert!(content.contains("Root Rules"));
    assert!(!content.contains("Hidden Rules"));
    assert!(!path.to_string_lossy().contains(".respondami"));
}

#[test]
fn load_agents_md_empty_file() {
    let dir = TempDir::new().unwrap();
    fs::write(dir.path().join("AGENTS.md"), "").unwrap();
    let result = load_agents_md(dir.path());
    assert!(result.is_ok());
    assert_eq!(result.unwrap(), None);
}

#[test]
fn load_agents_md_whitespace_only() {
    let dir = TempDir::new().unwrap();
    fs::write(dir.path().join("AGENTS.md"), "   \n\n  ").unwrap();
    let result = load_agents_md(dir.path());
    assert!(result.is_ok());
    assert_eq!(result.unwrap(), None);
}

#[test]
fn load_agents_md_read_error() {
    // On Unix, create a file with no read permissions.
    // On Windows, this test is skipped (permission model differs).
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;

        let dir = TempDir::new().unwrap();
        let path = dir.path().join("AGENTS.md");
        fs::write(&path, "secret").unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o000)).unwrap();
        let result = load_agents_md(dir.path());
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::PermissionDenied);
    }

    #[cfg(not(unix))]
    {
        // Placeholder — test runs but doesn't assert on non-Unix
        let dir = TempDir::new().unwrap();
        let _ = load_agents_md(dir.path());
    }
}

// ---------------------------------------------------------------------------
// GENERATE_PROMPT and GENERATE_CHAT_MESSAGE constants
// ---------------------------------------------------------------------------

#[test]
fn generate_prompt_is_non_empty() {
    assert!(!GENERATE_PROMPT.is_empty());
}

#[test]
fn generate_prompt_mentions_agents_md() {
    assert!(GENERATE_PROMPT.contains("AGENTS.md"));
}

#[test]
fn generate_prompt_mentions_architecture() {
    assert!(GENERATE_PROMPT.contains("Architecture"));
}

#[test]
fn generate_prompt_mentions_file_map() {
    assert!(GENERATE_PROMPT.contains("File Map"));
}

#[test]
fn generate_chat_message_has_emoji() {
    assert!(GENERATE_CHAT_MESSAGE.contains('🤖'));
}

#[test]
fn generate_chat_message_mentions_agents_md() {
    assert!(GENERATE_CHAT_MESSAGE.contains("AGENTS.md"));
}
