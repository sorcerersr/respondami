//! History prefix stability guard — detects request-to-request history drift.
//!
//! vLLM-style prefix caching only hits when the message prefix of consecutive
//! `/v1/chat/completions` requests is byte-identical and append-only. Any
//! re-serialization of an already-sent message (e.g. a session rebuild that
//! differs from the live agent context) silently breaks the KV-cache prefix.
//!
//! [`HistoryGuard`] compares per-message digests across outgoing requests and
//! reports the first divergence when the previous outgoing list is not a
//! prefix of the current one. It observes and never blocks: a violation only
//! produces a warning log entry.
//!
//! Digests are FNV-1a over the message's wire-relevant JSON. FNV-1a is
//! deliberately in-process and non-cryptographic: it is a change detector for
//! the current process, never persisted and never used for security.

use crate::session::AgentMessage;

/// FNV-1a 64-bit offset basis.
const FNV_OFFSET_BASIS: u64 = 0xcbf2_9ce4_8422_2325;
/// FNV-1a 64-bit prime.
const FNV_PRIME: u64 = 0x0000_0100_0000_01b3;

/// Digest of one outgoing message.
///
/// FNV-1a over the message's wire-relevant JSON (see [`message_wire_value`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MessageDigest(u64);

impl std::fmt::Display for MessageDigest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:#018x}", self.0)
    }
}

impl MessageDigest {
    /// Compute the FNV-1a digest of `bytes`.
    #[must_use]
    pub fn from_bytes(bytes: &[u8]) -> Self {
        let mut hash = FNV_OFFSET_BASIS;
        for &byte in bytes {
            hash ^= u64::from(byte);
            hash = hash.wrapping_mul(FNV_PRIME);
        }
        Self(hash)
    }
}

/// Wire-relevant view of a message: its JSON with the `usage` field removed.
///
/// `usage` is token bookkeeping only — it is never sent to the LLM — so it
/// must not influence the digest: a usage drift is not a cache break.
fn message_wire_value(msg: &AgentMessage) -> serde_json::Value {
    let mut value = serde_json::to_value(msg).unwrap_or(serde_json::Value::Null);
    if let serde_json::Value::Object(map) = &mut value
        && map.get("role").and_then(serde_json::Value::as_str) == Some("assistant")
    {
        map.remove("usage");
    }
    value
}

/// Compute the digest for one outgoing message.
///
/// The digest is stable for a message: two messages that serialize
/// identically (ignoring `usage`) produce the same digest.
#[must_use]
pub fn message_digest(msg: &AgentMessage) -> MessageDigest {
    let json = message_wire_value(msg).to_string();
    MessageDigest::from_bytes(json.as_bytes())
}

/// A detected prefix divergence between two consecutive outgoing requests.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HistoryViolation {
    /// Position in the current (raw) message list where the previous outgoing
    /// prefix diverges, or where the current list ends if it shrank.
    pub index: usize,
    /// `true` when the current list is shorter than the previous one.
    pub shrank: bool,
    /// Digest of the previous message at the divergence point, if any.
    pub previous: Option<MessageDigest>,
    /// Digest of the current message at the divergence point, if any.
    pub current: Option<MessageDigest>,
}

impl std::fmt::Display for HistoryViolation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        fn digest_or_dash(digest: Option<MessageDigest>) -> String {
            digest.map_or_else(|| "-".to_string(), |d| d.to_string())
        }
        if self.shrank {
            write!(
                f,
                "history shrank at message {} (previous digest {})",
                self.index,
                digest_or_dash(self.previous)
            )
        } else {
            write!(
                f,
                "history diverged at message {} (previous digest {}, current digest {})",
                self.index,
                digest_or_dash(self.previous),
                digest_or_dash(self.current)
            )
        }
    }
}

/// Per-message digests of the most recent outgoing request.
///
/// Entries are `Option` so callers can mark ephemeral messages — messages that
/// exist only in the live context and are never persisted, such as the
/// synthetic `hook_instruction` assistant/tool pair. Ephemeral entries are
/// skipped when comparing prefixes, so they cannot false-positive.
#[derive(Debug, Default)]
pub struct HistoryGuard {
    previous: Vec<Option<MessageDigest>>,
}

impl HistoryGuard {
    /// Create an empty guard (no outgoing request seen yet).
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Check this request's digests against the previous outgoing list.
    ///
    /// Stores `current` as the new baseline, then returns a [`HistoryViolation`]
    /// when the previous list — with ephemeral entries removed — is not a prefix
    /// of `current` — with ephemeral entries removed. An identical list is
    /// accepted: it is a retry of the same request (transient provider errors
    /// are retried with unchanged context).
    pub fn check_and_update(&mut self, current: &[Option<MessageDigest>]) -> Option<HistoryViolation> {
        let violation = self.violation(current);
        self.previous = current.to_vec();
        violation
    }

    /// Legitimate history rewrite: start fresh (new session, session resume,
    /// post-compaction restart).
    pub fn reset(&mut self) {
        self.previous.clear();
    }

    fn violation(&self, current: &[Option<MessageDigest>]) -> Option<HistoryViolation> {
        // Two-pointer walk over the raw lists, skipping ephemeral (None)
        // entries on both sides. The previous projection must be a prefix of
        // the current projection; pure appends are fine, so the walk stops
        // early when the previous list is exhausted.
        let mut prev_idx = 0;
        let mut curr_idx = 0;
        loop {
            while prev_idx < self.previous.len() && self.previous[prev_idx].is_none() {
                prev_idx += 1;
            }
            while curr_idx < current.len() && current[curr_idx].is_none() {
                curr_idx += 1;
            }
            match (prev_idx < self.previous.len(), curr_idx < current.len()) {
                (false, _) => return None, // Previous exhausted — pure append or retry.
                (true, false) => {
                    // Current list is shorter: history shrank without a reset.
                    return Some(HistoryViolation {
                        index: curr_idx,
                        shrank: true,
                        previous: self.previous[prev_idx],
                        current: None,
                    });
                }
                (true, true) => {
                    let prev_digest = self.previous[prev_idx].expect("checked non-None above");
                    let curr_digest = current[curr_idx].expect("checked non-None above");
                    if prev_digest != curr_digest {
                        return Some(HistoryViolation {
                            index: curr_idx,
                            shrank: false,
                            previous: Some(prev_digest),
                            current: Some(curr_digest),
                        });
                    }
                }
            }
            prev_idx += 1;
            curr_idx += 1;
        }
    }
}
