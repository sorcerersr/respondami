//! Activity indicator — character sweep animation for the status bar.
//!
//! Uses time-based ticking (100ms interval) so the animation rate is consistent
//! regardless of which loop drives it. Returns `Vec<Span>` for integration into
//! the status bar; the label is passed at render time.

use std::time::Instant;

use ratatui::style::{Color, Modifier, Style};
use ratatui::text::Span;

use super::theme::Theme;

/// Activity indicator — a character sweep animation rendered as status bar spans.
///
/// Uses time-based ticking (100ms interval) so the animation rate is consistent
/// regardless of which loop drives it (main loop ~50ms sleep, agent loop ~20ms timeout).
///
/// Unlike the old `WorkingIndicator` which owned a label and rendered a `Line`,
/// this struct only tracks sweep position. The label is passed at render time,
/// and it returns `Vec<Span>` for integration into the status bar.
#[derive(Debug)]
pub struct ActivityIndicator {
    pub(crate) sweep_pos: usize,
    pub(crate) sweep_dir: i32,
    pub(crate) last_tick: Instant,
}

impl ActivityIndicator {
    #[must_use]
    pub fn new() -> Self {
        Self {
            sweep_pos: 0,
            sweep_dir: 1,
            last_tick: Instant::now(),
        }
    }

    /// Advance the sweep by one step, bouncing at label boundaries.
    ///
    /// Only advances if at least 100ms has elapsed since the last tick,
    /// ensuring a consistent ~10 ticks/s rate regardless of loop speed.
    pub fn tick(&mut self, label: &str) {
        if self.last_tick.elapsed().as_millis() < 100 {
            return;
        }
        self.last_tick = Instant::now();
        let chars = label.chars().count().max(2);
        let new_pos = self.sweep_pos as i32 + self.sweep_dir;
        if new_pos >= chars as i32 - 1 {
            self.sweep_dir = -1;
            self.sweep_pos = chars - 1;
        } else if new_pos < 0 {
            self.sweep_dir = 1;
            self.sweep_pos = 0;
        } else {
            self.sweep_pos = new_pos as usize;
        }
    }

    /// Render label as styled spans with sweep animation.
    ///
    /// When `is_active` is false, renders all chars in `color` (static).
    #[must_use]
    pub fn render_spans(&self, label: &str, color: Color, theme: &Theme) -> Vec<Span<'_>> {
        let chars: Vec<char> = label.chars().collect();
        let len = chars.len().max(1);
        let mut spans = Vec::with_capacity(len);

        for (i, c) in chars.iter().enumerate() {
            let style = if i == self.sweep_pos {
                Style::default()
                    .fg(theme.accent)
                    .add_modifier(Modifier::BOLD)
            } else {
                let dist = i.abs_diff(self.sweep_pos);
                match dist {
                    1 => Style::default().fg(color),
                    2 => Style::default().fg(theme.text_muted),
                    _ => Style::default().fg(theme.text_dim),
                }
            };
            spans.push(Span::styled(c.to_string(), style));
        }

        spans
    }

    /// Render label as static spans (no sweep animation).
    #[must_use]
    pub fn render_static(&self, label: &str, color: Color, _theme: &Theme) -> Vec<Span<'_>> {
        let chars: Vec<char> = label.chars().collect();
        let mut spans = Vec::with_capacity(chars.len());
        for c in chars {
            spans.push(Span::styled(c.to_string(), Style::default().fg(color)));
        }
        spans
    }
}

impl Default for ActivityIndicator {
    fn default() -> Self {
        Self::new()
    }
}
