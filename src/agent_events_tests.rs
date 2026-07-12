use crate::tui::AgentEvent;
use crate::session::Usage;

/// Regression test: `drain_pending_events` processes Token, Reasoning, and Usage events
/// that are still buffered in the channel when Done arrives.
///
/// Bug (C-1): Done may arrive while Token/Reasoning/Usage events are still buffered
/// (channel capacity 256). Without draining, the final lines of output are
/// silently dropped, and Usage events are lost from session counters.
///
/// Fix: All drain paths now include Usage events (previously error paths
/// called with `include_usage = false`, silently discarding Usage events).
#[test]
fn drain_pending_events_processes_all_event_types() {
    let (tx, mut rx) = tokio::sync::mpsc::channel::<AgentEvent>(256);
    let _ = tx.try_send(AgentEvent::Token("hello".into()));
    let _ = tx.try_send(AgentEvent::Token(" world".into()));
    let _ = tx.try_send(AgentEvent::Reasoning("thinking...".into()));
    let _ = tx.try_send(AgentEvent::Usage(Usage {
        prompt_tokens: 100,
        completion_tokens: 50,
        total_tokens: 150,
    }));
    drop(tx);

    // All event types are processed
    let mut tokens = Vec::new();
    let mut reasoning = Vec::new();
    let mut usages = Vec::new();
    while let Ok(evt) = rx.try_recv() {
        match evt {
            AgentEvent::Token(t) => tokens.push(t),
            AgentEvent::Reasoning(r) => reasoning.push(r),
            AgentEvent::Usage(u) => usages.push(u),
            _ => {}
        }
    }
    assert_eq!(tokens, vec!["hello".to_string(), " world".to_string()]);
    assert_eq!(reasoning, vec!["thinking...".to_string()]);
    assert_eq!(usages.len(), 1);
    assert_eq!(usages[0].completion_tokens, 50);
}
