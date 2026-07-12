//! Context module — token rate tracking and provider usage correction.
//!
//! Provides `TokenRateTracker` for turn lifecycle management: start/pause/finalize,
//! char counting, provider usage correction, and session total persistence.

pub mod token_tracker;

#[cfg(test)]
mod token_tracker_tests;

pub use token_tracker::TokenRateTracker;
