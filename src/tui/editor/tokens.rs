//! Atomic image-token handling for the input buffer.
//!
//! A pasted/dropped image is represented in the buffer as an atomic token
//! `[img:N]` where `N` is a monotonic id into the editor's `pending_images`
//! side table. The token is a single editing unit: the cursor skips over it,
//! backspace/delete remove it whole, and wrapping never splits it. It is
//! stripped from the text at turn start, where the side-table images are
//! attached to the outgoing user message.
//!
//! These are pure string functions shared by the cursor, input layer, and
//! wrap modules.

use regex::Regex;
use std::sync::OnceLock;

/// Prefix of an image token.
pub const TOKEN_PREFIX: &str = "[img:";

/// A token's span and id within a buffer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TokenSpan {
    /// Byte offset of the token's start (`[`).
    pub start: usize,
    /// Byte offset just past the token's end (`]`).
    pub end: usize,
    /// The monotonic id (`N` in `[img:N]`).
    pub id: u32,
}

/// The token pattern: `[img:` + one or more digits + `]`.
fn token_regex() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"\[img:(\d+)\]").expect("static token regex is valid"))
}

/// Format a token for a given id.
#[must_use]
pub fn format_token(id: u32) -> String {
    format!("{TOKEN_PREFIX}{id}]")
}

/// Find all image tokens in `input`, in order.
#[must_use]
pub fn find_tokens(input: &str) -> Vec<TokenSpan> {
    token_regex()
        .captures_iter(input)
        .filter_map(|cap| {
            let id: u32 = cap.get(1)?.as_str().parse().ok()?;
            Some(TokenSpan {
                start: cap.get(0)?.start(),
                end: cap.get(0)?.end(),
                id,
            })
        })
        .collect()
}

/// The token that starts exactly at `pos`, if any.
#[must_use]
pub fn token_starting_at(input: &str, pos: usize) -> Option<TokenSpan> {
    find_tokens(input).into_iter().find(|t| t.start == pos)
}

/// The token that ends exactly at `pos`, if any.
#[must_use]
pub fn token_ending_at(input: &str, pos: usize) -> Option<TokenSpan> {
    find_tokens(input).into_iter().find(|t| t.end == pos)
}

/// Remove all image tokens from `input`, returning the cleaned text.
///
/// Used at turn start (the images are attached separately) and before pushing
/// to chat history.
#[must_use]
pub fn strip_tokens(input: &str) -> String {
    token_regex().replace_all(input, "").into_owned()
}
