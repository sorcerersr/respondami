//! Tests for `src/tui/token_stats.rs` — project token statistics computation.

use std::fs;
use std::path::PathBuf;

use super::token_stats::{compute_project_stats, ProjectTokenStats};
use crate::session::{AgentMessage, SessionStore, Usage};

fn temp_dir() -> PathBuf {
    let dir = std::env::temp_dir().join(format!("respondami_token_stats_{}", uuid::Uuid::new_v4()));
    fs::create_dir_all(&dir).unwrap();
    dir
}

// ---------------------------------------------------------------------------
// compute_project_stats
// ---------------------------------------------------------------------------

#[test]
fn returns_none_when_no_sessions_dir() {
    let dir = temp_dir();
    let result = compute_project_stats(&dir);
    assert!(result.is_none());
}

#[test]
fn returns_none_when_empty_sessions_dir() {
    let dir = temp_dir();
    fs::create_dir_all(dir.join(".respondami").join("sessions")).unwrap();
    let result = compute_project_stats(&dir);
    assert!(result.is_none());
}

#[test]
fn returns_none_when_no_jsonl_files() {
    let dir = temp_dir();
    let sessions_dir = dir.join(".respondami").join("sessions");
    fs::create_dir_all(&sessions_dir).unwrap();
    fs::write(sessions_dir.join("notes.txt"), "not a session").unwrap();
    let result = compute_project_stats(&dir);
    assert!(result.is_none());
}

#[test]
fn computes_stats_from_single_session() {
    let dir = temp_dir();
    let mut store = SessionStore::new(&dir);
    store.create_session("test-model".to_string(), 8192, "/test".to_string());

    let msg = AgentMessage::assistant(
        "response".to_string(),
        String::new(),
        Vec::new(),
        Some(Usage {
            prompt_tokens: 500_000,
            completion_tokens: 200_000,
            total_tokens: 700_000,
        }),
    );
    store.append_message(None, msg).unwrap();

    let stats = compute_project_stats(&dir).unwrap();
    assert_eq!(stats.session_count, 1);
    assert_eq!(stats.total_input_tokens, 500_000);
    assert_eq!(stats.total_output_tokens, 200_000);
    assert_eq!(stats.total_tokens(), 700_000);
}

#[test]
fn aggregates_across_multiple_sessions() {
    let dir = temp_dir();
    let sessions_dir = dir.join(".respondami").join("sessions");
    fs::create_dir_all(&sessions_dir).unwrap();

    // Session 1
    let mut store = SessionStore::new(&dir);
    store.create_session("test-model".to_string(), 8192, "/test".to_string());
    let msg = AgentMessage::assistant(
        "r1".to_string(),
        String::new(),
        Vec::new(),
        Some(Usage {
            prompt_tokens: 1_000_000,
            completion_tokens: 500_000,
            total_tokens: 1_500_000,
        }),
    );
    store.append_message(None, msg).unwrap();

    // Session 2
    store.create_session("test-model".to_string(), 8192, "/test".to_string());
    let msg = AgentMessage::assistant(
        "r2".to_string(),
        String::new(),
        Vec::new(),
        Some(Usage {
            prompt_tokens: 2_000_000,
            completion_tokens: 1_000_000,
            total_tokens: 3_000_000,
        }),
    );
    store.append_message(None, msg).unwrap();

    let stats = compute_project_stats(&dir).unwrap();
    assert_eq!(stats.session_count, 2);
    assert_eq!(stats.total_input_tokens, 3_000_000);
    assert_eq!(stats.total_output_tokens, 1_500_000);
    assert_eq!(stats.avg_input_per_session(), 1_500_000);
    assert_eq!(stats.avg_output_per_session(), 750_000);
}

#[test]
fn skips_assistant_messages_without_usage() {
    let dir = temp_dir();
    let mut store = SessionStore::new(&dir);
    store.create_session("test-model".to_string(), 8192, "/test".to_string());

    // Assistant message with no usage data
    let msg = AgentMessage::assistant("response".to_string(), String::new(), Vec::new(), None);
    store.append_message(None, msg).unwrap();

    let stats = compute_project_stats(&dir).unwrap();
    assert_eq!(stats.session_count, 1);
    assert_eq!(stats.total_input_tokens, 0);
    assert_eq!(stats.total_output_tokens, 0);
}

#[test]
fn skips_corrupted_lines() {
    let dir = temp_dir();
    let mut store = SessionStore::new(&dir);
    store.create_session("test-model".to_string(), 8192, "/test".to_string());

    // Append a valid assistant message
    let msg = AgentMessage::assistant(
        "response".to_string(),
        String::new(),
        Vec::new(),
        Some(Usage {
            prompt_tokens: 100,
            completion_tokens: 50,
            total_tokens: 150,
        }),
    );
    store.append_message(None, msg).unwrap();

    // Append corrupted line to the session file
    let session_path = store.sessions_dir().join(format!("{}.jsonl", store.session_id().unwrap()));
    let mut content = fs::read_to_string(&session_path).unwrap();
    content.push_str("this is not valid json\n");
    fs::write(&session_path, content).unwrap();

    // Should still succeed, skipping the corrupted line
    let stats = compute_project_stats(&dir).unwrap();
    assert_eq!(stats.session_count, 1);
    assert_eq!(stats.total_input_tokens, 100);
}

// ---------------------------------------------------------------------------
// ProjectTokenStats helpers
// ---------------------------------------------------------------------------

#[test]
fn format_millions_displays_two_decimals() {
    let s = ProjectTokenStats::format_millions(1_234_567);
    assert_eq!(s, "1.23M");
}

#[test]
fn format_millions_handles_zero() {
    let s = ProjectTokenStats::format_millions(0);
    assert_eq!(s, "0.00M");
}

#[test]
fn io_ratio_computes_correctly() {
    let stats = ProjectTokenStats {
        session_count: 3,
        total_input_tokens: 750_000,
        total_output_tokens: 250_000,
    };
    assert_eq!(stats.input_ratio(), 75);
    assert_eq!(stats.output_ratio(), 25);
}

#[test]
fn io_ratio_is_zero_when_no_tokens() {
    let stats = ProjectTokenStats {
        session_count: 1,
        total_input_tokens: 0,
        total_output_tokens: 0,
    };
    assert_eq!(stats.input_ratio(), 0);
    assert_eq!(stats.output_ratio(), 0);
}

#[test]
fn averages_are_zero_when_no_sessions() {
    let stats = ProjectTokenStats {
        session_count: 0,
        total_input_tokens: 100,
        total_output_tokens: 50,
    };
    assert_eq!(stats.avg_input_per_session(), 0);
    assert_eq!(stats.avg_output_per_session(), 0);
}
