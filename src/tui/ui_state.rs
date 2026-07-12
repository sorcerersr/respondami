use std::time::Instant;

use tachyonfx::EffectManager;

use super::activity_indicator::ActivityIndicator;

/// UI animation and display state.
#[derive(Debug)]
pub struct UIState {
    pub activity_indicator: ActivityIndicator,
    pub effect_manager: EffectManager<usize>,
    pub last_render_time: Instant,
    pub terminal_height: usize,
    pub terminal_width: usize,
    pub status_bar_message: Option<(String, Instant)>,
    pub should_quit: bool,
    /// Width of the editor input area in display columns. Set during layout pass.
    pub input_area_width: usize,
}

impl UIState {
    #[must_use]
    pub fn new() -> Self {
        Self {
            activity_indicator: ActivityIndicator::new(),
            effect_manager: EffectManager::default(),
            last_render_time: Instant::now(),
            terminal_height: 24,
            terminal_width: 80,
            status_bar_message: None,
            should_quit: false,
            input_area_width: 80,
        }
    }
}

impl Default for UIState {
    fn default() -> Self {
        Self::new()
    }
}
