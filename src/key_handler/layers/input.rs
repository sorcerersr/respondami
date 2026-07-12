//! Input layer — shared character insertion, cursor movement, and editing.
//!
//! Used by: Idle
//!
//! Configurable:
//! - Allow newlines (Shift+Enter / Alt+Enter)
//! - Enter sends (or does nothing)
//! - Character triggers (@)
//! - Backspace behavior
//!
//! Uses existing cursor functions from `crate::tui::editor`.

use crossterm::event::KeyEvent;
use crate::tui::editor::{
    cursor_backspace, cursor_delete, cursor_end, cursor_home, cursor_left,
    cursor_right, cursor_up_visual, cursor_down_visual,
};
use crate::tui::editor::wrap::{build_visual_lines, cursor_visual_pos};
use crate::tui::App;

/// Configuration for the `InputLayer`.
#[derive(Debug, Default)]
pub struct InputConfig {
    /// Allow Shift+Enter and Alt+Enter to insert newlines.
    pub allow_newlines: bool,
    /// Enable @ trigger (file autocomplete).
    pub enable_at_trigger: bool,
}

/// Callback for the @ trigger.
pub type AtTriggerCallback = Box<dyn FnOnce(&mut App)>;

/// Input layer that handles character insertion, cursor movement, and editing.
pub struct InputLayer {
    pub(crate) config: InputConfig,
    pub(crate) at_trigger: Option<AtTriggerCallback>,
}

impl std::fmt::Debug for InputLayer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("InputLayer")
            .field("config", &self.config)
            .field("at_trigger", &self.at_trigger.as_ref().map(|_| "Some(callback)"))
            .finish()
    }
}

impl InputLayer {
    /// Create a new `InputLayer` with the given configuration.
    pub fn new(config: InputConfig) -> Self {
        Self {
            config,
            at_trigger: None,
        }
    }

    /// Set the callback for @ trigger.
    pub fn with_at_trigger(mut self, cb: AtTriggerCallback) -> Self {
        self.at_trigger = Some(cb);
        self
    }

    /// Handle a key event. Returns `true` if the event was handled and caused a quit.
    /// Returns `false` if the event was handled but no quit, or if the event was not handled.
    ///
    /// The caller should only call this if the key is a character, cursor movement,
    /// Enter, Backspace, or Delete. State transitions (Esc, Ctrl+C, etc.) should
    /// be handled by the `StateTransitionLayer` first.
    pub fn handle(&mut self, app: &mut App, key: &KeyEvent) -> anyhow::Result<bool> {
        match key.code {
            crossterm::event::KeyCode::Enter => self.handle_enter(app, key.modifiers),
            crossterm::event::KeyCode::Char(c) => self.handle_char(app, c),
            crossterm::event::KeyCode::Backspace => {
                self.handle_backspace(app);
                Ok(false)
            }
            crossterm::event::KeyCode::Delete => {
                self.handle_delete(app);
                Ok(false)
            }
            crossterm::event::KeyCode::Left => {
                self.handle_left(app);
                Ok(false)
            }
            crossterm::event::KeyCode::Right => {
                self.handle_right(app);
                Ok(false)
            }
            crossterm::event::KeyCode::Up => {
                self.handle_up(app);
                Ok(false)
            }
            crossterm::event::KeyCode::Down => {
                self.handle_down(app);
                Ok(false)
            }
            crossterm::event::KeyCode::Home => {
                self.handle_home(app);
                Ok(false)
            }
            crossterm::event::KeyCode::End => {
                self.handle_end(app);
                Ok(false)
            }
            _ => Ok(false),
        }
    }

    fn handle_enter(
        &mut self,
        app: &mut App,
        modifiers: crossterm::event::KeyModifiers,
    ) -> anyhow::Result<bool> {
        // Shift+Enter / Alt+Enter: insert newline if allowed
        if key_allows_newline(&modifiers) && self.config.allow_newlines {
            app.editor.input_buffer.insert(app.editor.cursor_pos, '\n');
            app.editor.cursor_pos += 1;
            return Ok(false);
        }

        Ok(false)
    }

    fn handle_char(&mut self, app: &mut App, c: char) -> anyhow::Result<bool> {
        let char_width = c.len_utf8();
        app.editor.input_buffer.insert_str(
            app.editor.cursor_pos,
            c.encode_utf8(&mut [0; 4]),
        );
        app.editor.cursor_pos += char_width;

        // Check for triggers (only when cursor is at end)
        let at_end = app.editor.cursor_pos == app.editor.input_buffer.len();

        if self.config.enable_at_trigger && at_end && c == '@' {
            if let Some(trigger) = self.at_trigger.take() {
                trigger(app);
            }
        } else if self.config.enable_at_trigger {
            // Extract show_hidden from current autocomplete mode
            if let crate::tui::AutocompleteMode::File { matches, show_hidden, .. } = &app.editor.autocomplete_mode {
                if !matches.is_empty() {
                    // Update autocomplete matches as user types
                    use crate::tui::autocomplete::get_query_after_char;
                    let query = get_query_after_char(&app.editor.input_buffer, '@');
                    let new_matches = app.fuzzy_match_files(&query, *show_hidden);
                    app.editor.autocomplete_mode = crate::tui::AutocompleteMode::File {
                        matches: new_matches,
                        selected: 0,
                        scroll_offset: 0,
                        show_hidden: *show_hidden,
                    };
                }
            } else if let crate::tui::AutocompleteMode::None = app.editor.autocomplete_mode {
                // No file autocomplete active; do nothing here
            }
        }

        Ok(false)
    }

    fn handle_backspace(&self, app: &mut App) {
        cursor_backspace(&mut app.editor.input_buffer, &mut app.editor.cursor_pos);

        // Update autocomplete if @ trigger is enabled
        if self.config.enable_at_trigger && app.editor.input_buffer.contains('@') {
            use crate::tui::autocomplete::get_query_after_char;
            let query = get_query_after_char(&app.editor.input_buffer, '@');
            let show_hidden = match &app.editor.autocomplete_mode {
                crate::tui::AutocompleteMode::File { show_hidden, .. } => *show_hidden,
                _ => app.config.config.ui.file_show_hidden,
            };
            let matches = app.fuzzy_match_files(&query, show_hidden);
            app.editor.autocomplete_mode = crate::tui::AutocompleteMode::File {
                matches,
                selected: 0,
                scroll_offset: 0,
                show_hidden,
            };
        } else if self.config.enable_at_trigger {
            app.editor.autocomplete_mode = crate::tui::AutocompleteMode::None;
        }
    }

    fn handle_delete(&self, app: &mut App) {
        cursor_delete(&mut app.editor.input_buffer, &mut app.editor.cursor_pos);
    }

    fn handle_left(&self, app: &mut App) {
        app.editor.cursor_pos = cursor_left(&app.editor.input_buffer, app.editor.cursor_pos);
    }

    fn handle_right(&self, app: &mut App) {
        app.editor.cursor_pos = cursor_right(&app.editor.input_buffer, app.editor.cursor_pos);
    }

    fn handle_up(&self, app: &mut App) {
        let wrap_width = app.ui.input_area_width;
        let input = &app.editor.input_buffer;

        // Get current visual position
        let (visual_line, _) = cursor_visual_pos(input, app.editor.cursor_pos, wrap_width);

        // If on first visual line (or empty input), try history navigation
        if visual_line == 0 && app.history_message_count() > 0 {
            if app.editor.history_index == 0 {
                // First time entering history — save current input
                app.editor.saved_input = Some(app.editor.input_buffer.clone());
                app.editor.saved_cursor = app.editor.cursor_pos;
            }

            // Increment history index, bounded by message count
            if app.editor.history_index < app.history_message_count() {
                app.editor.history_index += 1;
                if let Some(msg) = app.get_history_message(app.editor.history_index - 1) {
                    app.editor.input_buffer = msg.to_string();
                    app.editor.cursor_pos = app.editor.input_buffer.len();
                }
            }
            // Scroll to bottom so input area is visible
            app.chat.auto_scroll = true;
            return;
        }

        // Normal visual-line cursor movement
        app.editor.cursor_pos = cursor_up_visual(input, app.editor.cursor_pos, wrap_width);
    }

    fn handle_down(&self, app: &mut App) {
        let wrap_width = app.ui.input_area_width;
        let input = &app.editor.input_buffer;

        // Get total visual lines
        let visual_lines = build_visual_lines(input, wrap_width);
        let total_visual = visual_lines.len().max(1);
        let (visual_line, _) = cursor_visual_pos(input, app.editor.cursor_pos, wrap_width);

        // If on last visual line and in history, navigate forward
        if visual_line >= total_visual.saturating_sub(1) && app.editor.history_index > 0 {
            app.editor.history_index -= 1;
            if app.editor.history_index == 0 {
                // Restore original input
                if let Some(saved) = app.editor.saved_input.take() {
                    app.editor.input_buffer = saved;
                    app.editor.cursor_pos = app.editor.saved_cursor;
                }
            } else {
                // Load previous history message
                if let Some(msg) = app.get_history_message(app.editor.history_index - 1) {
                    app.editor.input_buffer = msg.to_string();
                    app.editor.cursor_pos = app.editor.input_buffer.len();
                }
            }
            return;
        }

        // Normal visual-line cursor movement
        app.editor.cursor_pos = cursor_down_visual(input, app.editor.cursor_pos, wrap_width);
    }

    fn handle_home(&self, app: &mut App) {
        app.editor.cursor_pos = cursor_home(&app.editor.input_buffer, app.editor.cursor_pos);
    }

    fn handle_end(&self, app: &mut App) {
        app.editor.cursor_pos = cursor_end(&app.editor.input_buffer, app.editor.cursor_pos);
    }
}

/// Check if the key modifiers indicate a newline insertion (Shift+Enter or Alt+Enter).
pub(crate) fn key_allows_newline(modifiers: &crossterm::event::KeyModifiers) -> bool {
    modifiers.contains(crossterm::event::KeyModifiers::SHIFT)
        || modifiers.contains(crossterm::event::KeyModifiers::ALT)
}
