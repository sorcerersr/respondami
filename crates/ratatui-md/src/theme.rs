//! Theme trait for markdown rendering.
//!
//! Implement this trait on your application's theme struct to provide styling
//! for markdown content blocks.

use ratatui::style::{Color, Style};

/// Trait for providing styles to the markdown renderer.
///
/// Implement this on your application's theme struct. The implementation
/// delegates to existing style methods.
pub trait MdTheme {
    // ── Parsing ──

    /// Style for normal paragraph text.
    fn text_style(&self) -> Style;

    /// Style for headings.
    fn heading_style(&self) -> Style;

    /// Muted text color (used for strikethrough).
    fn text_muted_color(&self) -> Color;

    /// Style for links.
    fn link_style(&self) -> Style;

    /// Style for inline code.
    fn inline_code_style(&self) -> Style;

    // ── Rendering ──

    /// Style for list bullet points.
    fn list_bullet_style(&self) -> Style;

    /// Background style for code blocks.
    fn code_block_style(&self) -> Style;

    /// Dim text color (used for borders, separators, code block titles).
    fn text_dim_color(&self) -> Color;

    /// Muted text style (used for block quotes).
    fn text_muted_style(&self) -> Style;
}
