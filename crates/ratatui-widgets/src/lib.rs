//! Reusable ratatui widgets extracted from the Respondami TUI.
//!
//! This crate contains widgets that are free from application logic and can
//! be used in other ratatui applications.
//!
//! # Widgets
//!
//! - [`FilledHeaderBar`] — A bordered block with a filled header row and content area,
//!   supporting expanded/collapsed display modes.
//! - [`PanelOverlay`] — A generic overlay panel with title bar, borders, and content lines.
//! - [`CommandPaletteOverlay`] — A command palette with search input and command list.
//! - [`PromptEditorOverlay`] — A full-screen text editor popup with cursor support.
//! - [`AutocompletePopup`] — A dropdown list positioned near a cursor column.

mod filled_header_bar;
mod panel_overlay;
mod command_palette_overlay;
mod prompt_editor_overlay;
mod autocomplete_popup;

#[doc(inline)]
pub use filled_header_bar::FilledHeaderBar;
#[doc(inline)]
pub use panel_overlay::PanelOverlay;
#[doc(inline)]
pub use command_palette_overlay::CommandPaletteOverlay;
#[doc(inline)]
pub use prompt_editor_overlay::PromptEditorOverlay;
#[doc(inline)]
pub use autocomplete_popup::AutocompletePopup;
