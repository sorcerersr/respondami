//! Chat display state — messages, scrolling, viewport dimensions.
//!
//! Tracks `chat_messages`, `scroll_offset`, `auto_scroll`, and `pinned_scroll`
//! for top-down scroll model (`scroll_offset=0` means viewport at top).

use super::messages::ChatMessage;

/// Chat display state: messages, scrolling, viewport dimensions.
#[derive(Debug)]
pub struct ChatState {
    pub chat_messages: Vec<ChatMessage>,
    pub scroll_offset: usize,
    pub last_total_height: usize,
    pub last_viewport_height: usize,
    pub auto_scroll: bool,
    pub pinned_scroll: bool,
}

impl ChatState {
    #[must_use]
    pub fn new() -> Self {
        Self {
            chat_messages: Vec::new(),
            scroll_offset: 0,
            last_total_height: 0,
            last_viewport_height: 0,
            auto_scroll: true,
            pinned_scroll: false,
        }
    }

    /// Threshold (in rows) from bottom to resume auto-scroll.
    /// When user scrolls within this many rows of bottom, `pinned_scroll` is cleared.
    const AUTO_SCROLL_THRESHOLD: usize = 3;

    /// Compute the maximum scroll offset (clamped to content bounds).
    #[must_use]
    fn get_max_offset(&self) -> usize {
        self.last_total_height.saturating_sub(self.last_viewport_height)
    }

    /// Scroll chat up by n lines (show earlier content).
    /// During streaming, user scroll up stops auto-scroll.
    pub fn scroll_up(&mut self, n: usize) {
        self.scroll_offset = self.scroll_offset.saturating_sub(n);
        self.auto_scroll = false;
        self.pinned_scroll = true;
    }

    /// Scroll chat down by n lines (reveal more at bottom).
    /// During streaming, reaching within `AUTO_SCROLL_THRESHOLD` of bottom resumes auto-scroll.
    pub fn scroll_down(&mut self, n: usize) {
        let max_offset = self.get_max_offset();
        self.scroll_offset = (self.scroll_offset + n).min(max_offset);
        // Resume auto-scroll when within threshold of bottom
        if self.is_at_bottom(Self::AUTO_SCROLL_THRESHOLD) {
            self.pinned_scroll = false;
            self.auto_scroll = true;
        }
    }

    /// Reset scroll to bottom.
    pub fn scroll_to_bottom(&mut self) {
        self.scroll_offset = self.get_max_offset();
    }

    /// Check if the viewport is at the bottom. Used for auto-scroll during streaming.
    /// Accepts a threshold (in rows) — returns true when within `threshold` rows of bottom.
    /// Use 0 for exact bottom check. Returns true when there's no content (matches pre-render behavior).
    #[must_use]
    pub fn is_at_bottom(&self, threshold: usize) -> bool {
        if self.last_total_height == 0 {
            return true;
        }
        self.scroll_offset + self.last_viewport_height >= self.last_total_height.saturating_sub(threshold)
    }
}

impl Default for ChatState {
    fn default() -> Self {
        Self::new()
    }
}
