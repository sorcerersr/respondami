//! Session persistence and context management.
//!
//! Manages session lifecycle (create, load, list), JSONL persistence,
//! context building, and compaction.
//!
//! Rust guideline compliant 2026-02-21

pub mod compaction;
pub mod display_adapter;
pub mod entry;
pub mod manager;

#[doc(inline)]
pub use compaction::{CompactionEngine, CompactionPlan, CompactionSettings};
#[doc(inline)]
pub use display_adapter::SessionDisplayAdapter;
#[doc(inline)]
pub use entry::{AgentMessage, ContentBlock, RequestTokenUsage, SessionEntry, TokenRateEntry, ToolCall, Usage};
#[doc(inline)]
pub use manager::{SessionStore, SessionMeta};

#[cfg(test)]
mod compaction_tests;
#[cfg(test)]
mod entry_tests;
#[cfg(test)]
mod manager_tests;
