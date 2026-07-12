//! Composable key handling layers.
//!
//! Each layer is an independent, reusable struct that handles a specific
//! aspect of key handling. Handlers compose layers instead of implementing
//! logic from scratch.
//!
//! Layers:
//! - `InputLayer` — character insertion, cursor movement, editing
//! - `NavigationLayer` — list navigation (Up/Down/j/k/Tab)
//! - `StateTransitionLayer` — Enter, Esc, Ctrl+C state transitions
//! - `ModalLayer` — modal-aware global shortcuts
//!
//! Rust guideline compliant 2026-02-21

mod input;
mod navigation;
mod transitions;
mod modal;

#[doc(inline)]
pub use input::{InputLayer, InputConfig};
#[doc(inline)]
pub use navigation::NavigationLayer;
#[doc(inline)]
pub use transitions::{StateTransitionLayer, TransitionAction};
#[doc(inline)]
pub use modal::ModalLayer;

#[cfg(test)]
mod input_tests;
#[cfg(test)]
mod modal_tests;
#[cfg(test)]
mod navigation_tests;
#[cfg(test)]
mod transitions_tests;
