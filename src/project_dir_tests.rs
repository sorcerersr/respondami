//! Tests for `src/project_dir.rs` — `.respondami/` directory + gitignore creation.

use std::fs;

use tempfile::TempDir;

use super::project_dir::{ensure_respondami_dir, GITIGNORE_CONTENT};

#[test]
fn creates_dir_and_gitignore() {
    let tmp = TempDir::new().unwrap();
    let cwd = tmp.path();

    ensure_respondami_dir(cwd);

    assert!(cwd.join(".respondami").is_dir());
    let gitignore = cwd.join(".respondami").join(".gitignore");
    assert!(gitignore.is_file());
    assert_eq!(fs::read_to_string(&gitignore).unwrap(), GITIGNORE_CONTENT);
}

#[test]
fn idempotent_second_call() {
    let tmp = TempDir::new().unwrap();
    let cwd = tmp.path();

    ensure_respondami_dir(cwd);
    let gitignore = cwd.join(".respondami").join(".gitignore");
    let first = fs::read_to_string(&gitignore).unwrap();

    ensure_respondami_dir(cwd);

    assert_eq!(fs::read_to_string(&gitignore).unwrap(), first);
    assert_eq!(first, GITIGNORE_CONTENT);
}

#[test]
fn never_overwrites_existing_gitignore() {
    let tmp = TempDir::new().unwrap();
    let cwd = tmp.path();

    let dir = cwd.join(".respondami");
    fs::create_dir_all(&dir).unwrap();
    let custom = "custom-content\n";
    fs::write(dir.join(".gitignore"), custom).unwrap();

    ensure_respondami_dir(cwd);

    assert_eq!(fs::read_to_string(dir.join(".gitignore")).unwrap(), custom);
}

#[test]
fn existing_dir_without_gitignore() {
    let tmp = TempDir::new().unwrap();
    let cwd = tmp.path();

    let sessions = cwd.join(".respondami").join("sessions");
    fs::create_dir_all(&sessions).unwrap();
    fs::write(sessions.join("session.jsonl"), "{}").unwrap();

    ensure_respondami_dir(cwd);

    let gitignore = cwd.join(".respondami").join(".gitignore");
    assert!(gitignore.is_file());
    assert_eq!(fs::read_to_string(&gitignore).unwrap(), GITIGNORE_CONTENT);
    // Pre-existing content untouched
    assert!(sessions.join("session.jsonl").is_file());
}
