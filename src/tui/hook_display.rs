//! Hook display mode configuration.
//!
//! Defines `HookDisplay` enum with three modes: Hidden (invisible), Minimal
//! (single dimmed line), and Full (expanded purple-bordered box). Supports
//! serde serialization for config persistence.

use serde::{Deserialize, Serialize};

/// Display mode for hook messages.
///
/// - **Hidden**: Hook execution is completely invisible in the chat area.
/// - **Minimal**: Display a single dimmed line with status icon and hook origin.
/// - **Full**: Expanded purple-bordered box with full output (always expanded).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HookDisplay {
    Hidden,
    #[default]
    Minimal,
    Full,
}

impl HookDisplay {
    /// Toggle through all modes: Hidden → Minimal → Full → Hidden.
    #[must_use]
    pub fn toggle(&self) -> Self {
        match self {
            Self::Hidden => Self::Minimal,
            Self::Minimal => Self::Full,
            Self::Full => Self::Hidden,
        }
    }

    /// Unicode icon to display for this mode.
    #[must_use]
    pub fn icon(&self) -> &'static str {
        match self {
            Self::Hidden => "●",
            Self::Minimal => "●",
            Self::Full => "▲",
        }
    }

    /// String representation for palette display.
    fn as_str(&self) -> &'static str {
        match self {
            Self::Hidden => "hidden",
            Self::Minimal => "minimal",
            Self::Full => "full",
        }
    }

    /// Display string for the command palette: "[current → next]".
    #[must_use]
    pub fn palette_mode_display(&self) -> String {
        let next = self.toggle();
        format!("[{} → {}]", self.as_str(), next.as_str())
    }
}
