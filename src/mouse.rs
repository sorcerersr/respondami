//! Mouse scroll support for the TUI.
//!
//! Enables button-event mouse tracking (`?1002h`) to capture scroll wheel and
//! button events. Does not enable any-motion tracking, so casual mouse movement
//! generates no events. Shift+drag bypasses capture for native text selection.

use std::fmt;

/// Lines to scroll per mouse wheel tick.
/// Matches standard TUI convention (less, nvim, htop).
pub const SCROLL_LINES: usize = 3;

/// Enable button-event mouse tracking (`?1002h`).
///
/// Sends scroll wheel + button events to the application.
/// Does NOT enable any-motion tracking (`?1003h`), so casual
/// mouse movement generates no events.
///
/// Shift+drag bypasses mouse capture in most terminal emulators,
/// allowing native text selection to work alongside scroll.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EnableMouseScroll;

impl crossterm::Command for EnableMouseScroll {
    fn write_ansi(&self, f: &mut impl fmt::Write) -> fmt::Result {
        // `?1002h` = button-event tracking; `?1006h` = SGR encoding. SGR mode is
        // required because the vte-based input reader (src/input) parses mouse
        // events as numeric-param CSI; the legacy normal encoding (`ESC [ M CB
        // Cx Cy`) is not a well-formed CSI sequence and vte cannot parse it.
        f.write_str("\x1b[?1002h\x1b[?1006h")
    }
}

/// Disable button-event mouse tracking (`?1002l`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DisableMouseScroll;

impl crossterm::Command for DisableMouseScroll {
    fn write_ansi(&self, f: &mut impl fmt::Write) -> fmt::Result {
        f.write_str("\x1b[?1006l\x1b[?1002l")
    }
}
