//! TUI state, rendering, and input handling.
//!
//! Contains the app state struct, chat rendering, editor, layout, themes,
//! and all UI-related modules.
//!
//! Rust guideline compliant 2026-02-21

pub mod agent_event;
pub mod agent_state;
pub mod app;
pub mod autocomplete;
pub mod chat;
pub mod chat_state;
pub mod config_state;
pub mod editor;
pub mod editor_state;
pub mod hook_display;
pub mod layout;
pub mod messages;
pub mod modal_state;
pub mod mode;
pub mod session_state;
pub mod status_bar;
pub mod thinking_display;
pub mod theme;
pub mod ui_state;
pub mod activity_indicator;

#[doc(inline)]
pub use agent_event::{AgentEvent, AbortReason, CompactionReason, PartialToolCall};
#[doc(inline)]
pub use app::App;
#[doc(inline)]
pub use hook_display::HookDisplay;
#[doc(inline)]
pub use mode::{AppState, AutocompleteMode};
#[doc(inline)]
pub use thinking_display::ThinkingDisplay;
#[doc(inline)]
pub use messages::ChatMessage;
#[doc(inline)]
pub use editor::{FileDiscovery, parse_file_references};
#[doc(inline)]
pub use layout::LayoutRenderer;
#[doc(inline)]
pub use theme::Theme;
#[doc(inline)]
pub use activity_indicator::ActivityIndicator;

#[cfg(test)]
mod activity_indicator_tests;
#[cfg(test)]
mod app_tests;
#[cfg(test)]
mod autocomplete_tests;
#[cfg(test)]
mod hook_display_tests;
#[cfg(test)]
mod layout_tests;
#[cfg(test)]
mod status_bar_tests;
#[cfg(test)]
mod thinking_display_tests;
