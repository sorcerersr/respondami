//! End-to-end tests for the VCC compaction strategy.

use super::vcc::VccCompaction;
use super::CompactionStrategy;
use crate::provider::{ChatChunk, ChatRequest, CompletionResponse, Provider, ProviderError, ProviderTrait};
use crate::session::{AgentMessage, SessionEntry};
use async_trait::async_trait;

/// A provider that never streams (VCC is LLM-free, so it is never called).
struct NoopProvider;

#[async_trait]
impl ProviderTrait for NoopProvider {
    fn clone_box(&self) -> Box<dyn ProviderTrait> {
        Box::new(NoopProvider)
    }
    async fn stream_chat(
        &self,
        _req: &ChatRequest,
        _tx: tokio::sync::mpsc::Sender<ChatChunk>,
        _cancel_rx: tokio::sync::watch::Receiver<bool>,
    ) -> Result<(), ProviderError> {
        unimplemented!("NoopProvider does not stream")
    }
    async fn complete(&self, _req: &ChatRequest) -> Result<CompletionResponse, ProviderError> {
        unimplemented!("NoopProvider does not complete")
    }
    async fn ping(&self) -> Result<(), ProviderError> {
        Ok(())
    }
}

fn provider() -> Provider {
    Provider::from_trait(Box::new(NoopProvider))
}

fn session_with_turns(n: usize) -> Vec<SessionEntry> {
    let mut entries = vec![SessionEntry::new_session("s".into(), "t".into(), "/".into(), "m".into(), 4096)];
    for i in 0..n {
        entries.push(SessionEntry::new_message(None, AgentMessage::user(format!("user turn {i}"))));
        entries.push(SessionEntry::new_message(None, AgentMessage::assistant(format!("reply {i}"), String::new(), vec![], None)));
    }
    entries
}

#[tokio::test]
async fn vcc_compute_produces_summary_and_cut() {
    let entries = session_with_turns(5);
    let config = crate::config::Config::default();
    let plan = VccCompaction
        .compute(provider(), config, entries, std::env::current_dir().unwrap(), vec![], None)
        .await
        .unwrap();
    assert_ne!(plan.summary, String::new());
    assert!(plan.cut_index > 0);
    assert!(plan.tokens_before > 0);
}

#[tokio::test]
async fn vcc_compute_keep_zero_compact_all() {
    let entries = session_with_turns(5);
    let config = crate::config::Config::default();
    let plan = VccCompaction
        .compute(provider(), config, entries, std::env::current_dir().unwrap(), vec![], Some(0))
        .await
        .unwrap();
    // Compact-all: cut at the end (sentinel handled by apply_compaction).
    assert!(plan.cut_index >= 10);
}

#[tokio::test]
async fn vcc_compute_too_few_is_noop() {
    let entries = session_with_turns(1);
    let config = crate::config::Config::default();
    let result = VccCompaction
        .compute(provider(), config, entries, std::env::current_dir().unwrap(), vec![], None)
        .await;
    let _ = result.unwrap_err();
}

#[tokio::test]
async fn vcc_compute_merges_previous_compaction() {
    let mut entries = session_with_turns(3);
    let kept_id = entries[1].id();
    let prev = "[Session Goal]\n- old goal\n\n---\n\n[user] old line";
    entries.push(SessionEntry::new_compaction(None, prev.to_string(), kept_id, 100));
    entries.push(SessionEntry::new_message(None, AgentMessage::user("new turn".to_string())));
    entries.push(SessionEntry::new_message(None, AgentMessage::assistant("new reply".to_string(), String::new(), vec![], None)));

    let config = crate::config::Config::default();
    let plan = VccCompaction
        .compute(provider(), config, entries, std::env::current_dir().unwrap(), vec![], None)
        .await
        .unwrap();
    assert!(plan.summary.contains("old goal") || plan.summary.contains("new turn"));
}

#[test]
fn vcc_config_defaults() {
    let config = crate::config::Config::default();
    assert_eq!(config.compaction.mode, crate::config::CompactionMode::Vcc);
    assert!(config.compaction.vcc.smart_keep_tail);
    assert_eq!(config.compaction.vcc.brief_budget_tokens, 1100);
}
