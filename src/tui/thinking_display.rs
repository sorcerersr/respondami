//! Thinking display mode configuration.
//!
//! Defines `ThinkingDisplay` enum with three modes: Hidden (no shortcut), Collapsed
//! ("Thinking... ▼", Ctrl+O toggles), and Expanded (header + recent thinking lines).
//! Supports serde serialization for config persistence.

use serde::{Deserialize, Serialize};

/// Display mode for thinking/reasoning blocks.
///
/// - **Hidden**: "Thinking..." only, no keyboard shortcut interaction.
/// - **Collapsed**: "Thinking... ▼", Ctrl+O toggles to expanded.
/// - **Expanded**: Header + up to N lines of recent thinking text. Ctrl+O toggles to collapsed.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ThinkingDisplay {
    Hidden,
    #[default]
    Collapsed,
    Expanded,
}

impl ThinkingDisplay {
    /// Toggle through all modes: Hidden → Collapsed → Expanded → Hidden.
    #[must_use]
    pub fn toggle(&self) -> Self {
        match self {
            Self::Hidden => Self::Collapsed,
            Self::Collapsed => Self::Expanded,
            Self::Expanded => Self::Hidden,
        }
    }

    /// Unicode icon to display alongside "Thinking..." for this mode.
    #[must_use]
    pub fn icon(&self) -> &'static str {
        match self {
            Self::Hidden => "",
            Self::Collapsed => "▼",
            Self::Expanded => "▲",
        }
    }

    /// String representation for palette display.
    fn as_str(&self) -> &'static str {
        match self {
            Self::Hidden => "hidden",
            Self::Collapsed => "collapsed",
            Self::Expanded => "expanded",
        }
    }

    /// Display string for the command palette: "[current → next]".
    #[must_use]
    pub fn palette_mode_display(&self) -> String {
        let next = self.toggle();
        format!("[{} → {}]", self.as_str(), next.as_str())
    }
}
