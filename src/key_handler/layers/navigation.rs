//! Navigation layer — shared list navigation (Up/Down/j/k/Tab/Shift+Tab).
//!
//! Used by: `CommandPalette`, `SlashCommandPalette`, `SessionSelect`, `InitPopup`
//!
//! Configurable:
//! - Item count getter
//! - Selected index getter/setter
//! - Execute callback (Enter)

use crossterm::event::KeyEvent;
use crate::tui::App;

/// Callback to get the total number of items in the list.
pub type ItemCountGetter = dyn Fn(&App) -> usize;

/// Callback to get the current selected index.
pub type SelectedGetter = dyn Fn(&App) -> usize;

/// Callback to set the selected index.
pub type SelectedSetter = dyn Fn(&mut App, usize);

/// Navigation layer that handles Up/Down/j/k/Tab/Shift+Tab list navigation.
pub struct NavigationLayer {
    item_count: Box<ItemCountGetter>,
    selected_getter: Box<SelectedGetter>,
    selected_setter: Box<SelectedSetter>,
}

impl std::fmt::Debug for NavigationLayer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NavigationLayer")
            .field("item_count", &"<closure>")
            .field("selected_getter", &"<closure>")
            .field("selected_setter", &"<closure>")
            .finish()
    }
}

impl NavigationLayer {
    /// Create a new `NavigationLayer`.
    pub fn new(
        item_count: Box<ItemCountGetter>,
        selected_getter: Box<SelectedGetter>,
        selected_setter: Box<SelectedSetter>,
    ) -> Self {
        Self {
            item_count,
            selected_getter,
            selected_setter,
        }
    }

    /// Handle a key event. Returns `true` if the event caused a quit.
    pub fn handle(&mut self, app: &mut App, key: &KeyEvent) -> anyhow::Result<bool> {
        match key.code {
            crossterm::event::KeyCode::Up | crossterm::event::KeyCode::Char('k') => {
                self.navigate_up(app);
                Ok(false)
            }
            crossterm::event::KeyCode::Down | crossterm::event::KeyCode::Char('j') => {
                self.navigate_down(app);
                Ok(false)
            }
            crossterm::event::KeyCode::Tab
                if key.modifiers.contains(crossterm::event::KeyModifiers::SHIFT) =>
            {
                self.navigate_up(app);
                Ok(false)
            }
            crossterm::event::KeyCode::Tab => {
                // Tab cycles down (same as Shift+Tab in current code, but we'll make it cycle down)
                self.navigate_down(app);
                Ok(false)
            }
            _ => Ok(false),
        }
    }

    fn navigate_up(&self, app: &mut App) {
        let count = (self.item_count)(app);
        if count == 0 {
            return;
        }
        let current = (self.selected_getter)(app);
        let new_index = if current > 0 {
            current - 1
        } else {
            count - 1
        };
        (self.selected_setter)(app, new_index);
    }

    fn navigate_down(&self, app: &mut App) {
        let count = (self.item_count)(app);
        if count == 0 {
            return;
        }
        let current = (self.selected_getter)(app);
        let new_index = if current + 1 >= count { 0 } else { current + 1 };
        (self.selected_setter)(app, new_index);
    }
}
