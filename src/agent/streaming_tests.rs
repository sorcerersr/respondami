// Tests for src/agent/streaming.rs

use super::streaming::OneshotGuard;
use crate::provider::ProviderError;
use tokio::sync::oneshot;

// ---------------------------------------------------------------------------
// OneshotGuard — panic-safety of the provider-task error channel (L-15)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn oneshot_guard_dropped_unsent_yields_error() {
    // Regression (L-15): if the provider task panics, the sender is dropped
    // unsent. Without the guard the receiver's `await` returns Err(RecvError),
    // which the streaming loop's `if let Ok(Err(e))` skips — a silent success.
    // With the guard, Drop sends a sentinel IncompleteStream error.
    let (tx, rx) = oneshot::channel::<Result<(), ProviderError>>();
    {
        let guard = OneshotGuard::new(tx);
        // Simulate a panic: drop the guard without taking/sending.
        drop(guard);
    };
    let result = rx.await;
    assert!(
        result.is_ok(),
        "receiver must get a value, not RecvError: {result:?}"
    );
    let inner = result.unwrap();
    assert!(
        matches!(inner, Err(ProviderError::IncompleteStream { .. })),
        "must be the sentinel IncompleteStream error, got: {inner:?}"
    );
}

#[tokio::test]
async fn oneshot_guard_real_send_wins_over_drop() {
    // A real send takes the sender out of the guard, so Drop must not
    // double-send. The receiver gets exactly the real result.
    let (tx, rx) = oneshot::channel::<Result<(), ProviderError>>();
    let mut guard = OneshotGuard::new(tx);
    let real = Err(ProviderError::NotConfigured("test".to_string()));
    if let Some(sender) = guard.take() {
        let _ = sender.send(real);
    }
    drop(guard);
    let result = rx.await.unwrap();
    assert!(
        matches!(result, Err(ProviderError::NotConfigured { .. })),
        "must be the real error, got: {result:?}"
    );
}

#[tokio::test]
async fn oneshot_guard_ok_result_propagates() {
    // The success path is unaffected: an Ok result is delivered as-is.
    let (tx, rx) = oneshot::channel::<Result<(), ProviderError>>();
    let mut guard = OneshotGuard::new(tx);
    if let Some(sender) = guard.take() {
        let _ = sender.send(Ok(()));
    }
    drop(guard);
    rx.await.unwrap().unwrap();
}
