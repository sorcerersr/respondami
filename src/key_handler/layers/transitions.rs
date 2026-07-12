//! State transition layer — Enter, Esc, Ctrl+C, and other state transitions.
//!
//! Each state composes its own transitions via callbacks.
//!
//! Used by: all state handlers
//!
//! The layer returns a `TransitionAction` that signals what action the
//! outer handler should take. This avoids needing access to `terminal`
//! inside callbacks (e.g., `start_turn` requires `terminal`).

use crossterm::event::KeyEvent;
use crate::tui::App;

#[derive(Debug, PartialEq)]
/// Actions that the outer handler should execute after a transition.
pub enum TransitionAction {
    /// Send the message (requires terminal).
    Send,
    /// Run a turn with a specific input string (requires terminal).
    RunTurnWithInput(String),
    /// No special action needed, just handled a state transition.
    None,
}

/// Callback for Enter key.
/// Returns `TransitionAction` that the outer handler should execute.
pub type EnterCallback = Box<dyn FnOnce(&mut App) -> TransitionAction>;

/// Callback for Esc key.
pub type EscCallback = Box<dyn FnOnce(&mut App)>;

/// Callback for Ctrl+C.
pub type CtrlCCallback = Box<dyn FnOnce(&mut App)>;

/// Callback for Ctrl+K.
pub type CtrlKCallback = Box<dyn FnOnce(&mut App)>;

/// Callback for Alt+Enter.
pub type AltEnterCallback = Box<dyn FnOnce(&mut App)>;

/// Callback for Shift+Enter.
pub type ShiftEnterCallback = Box<dyn FnOnce(&mut App)>;

/// Callback for Ctrl+G.
pub type CtrlGCallback = Box<dyn FnOnce(&mut App)>;

/// State transition layer that handles Enter, Esc, Ctrl+C, and other state transitions.
pub struct StateTransitionLayer {
    enter: Option<EnterCallback>,
    esc: Option<EscCallback>,
    ctrl_c: Option<CtrlCCallback>,
    ctrl_k: Option<CtrlKCallback>,
    alt_enter: Option<AltEnterCallback>,
    shift_enter: Option<ShiftEnterCallback>,
    ctrl_g: Option<CtrlGCallback>,
}

impl std::fmt::Debug for StateTransitionLayer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("StateTransitionLayer")
            .field("enter", &self.enter.as_ref().map(|_| "Some(callback)"))
            .field("esc", &self.esc.as_ref().map(|_| "Some(callback)"))
            .field("ctrl_c", &self.ctrl_c.as_ref().map(|_| "Some(callback)"))
            .field("ctrl_k", &self.ctrl_k.as_ref().map(|_| "Some(callback)"))
            .field("alt_enter", &self.alt_enter.as_ref().map(|_| "Some(callback)"))
            .field("shift_enter", &self.shift_enter.as_ref().map(|_| "Some(callback)"))
            .field("ctrl_g", &self.ctrl_g.as_ref().map(|_| "Some(callback)"))
            .finish()
    }
}

impl StateTransitionLayer {
    /// Create a new `StateTransitionLayer` with no callbacks.
    pub fn new() -> Self {
        Self {
            enter: None,
            esc: None,
            ctrl_c: None,
            ctrl_k: None,
            alt_enter: None,
            shift_enter: None,
            ctrl_g: None,
        }
    }

    /// Set the Enter callback.
    pub fn with_enter(mut self, cb: EnterCallback) -> Self {
        self.enter = Some(cb);
        self
    }

    /// Set the Esc callback.
    pub fn with_esc(mut self, cb: EscCallback) -> Self {
        self.esc = Some(cb);
        self
    }

    /// Set the Ctrl+C callback.
    pub fn with_ctrl_c(mut self, cb: CtrlCCallback) -> Self {
        self.ctrl_c = Some(cb);
        self
    }

    /// Set the Ctrl+K callback.
    pub fn with_ctrl_k(mut self, cb: CtrlKCallback) -> Self {
        self.ctrl_k = Some(cb);
        self
    }

    /// Set the Alt+Enter callback.
    pub fn with_alt_enter(mut self, cb: AltEnterCallback) -> Self {
        self.alt_enter = Some(cb);
        self
    }

    /// Set the Shift+Enter callback.
    pub fn with_shift_enter(mut self, cb: ShiftEnterCallback) -> Self {
        self.shift_enter = Some(cb);
        self
    }

    /// Set the Ctrl+G callback.
    pub fn with_ctrl_g(mut self, cb: CtrlGCallback) -> Self {
        self.ctrl_g = Some(cb);
        self
    }

    /// Handle a key event. Returns `Some(action)` if the event was handled and an action
    /// should be executed, `None` if the event was not handled.
    pub fn handle(&mut self, app: &mut App, key: &KeyEvent) -> Option<TransitionAction> {
        // Ctrl+G
        if key.code == crossterm::event::KeyCode::Char('g')
            && key.modifiers.contains(crossterm::event::KeyModifiers::CONTROL)
            && let Some(cb) = self.ctrl_g.take()
        {
            cb(app);
            return Some(TransitionAction::None);
        }

        // Ctrl+C
        if key.code == crossterm::event::KeyCode::Char('c')
            && key.modifiers.contains(crossterm::event::KeyModifiers::CONTROL)
            && let Some(cb) = self.ctrl_c.take()
        {
            cb(app);
            return Some(TransitionAction::None);
        }

        // Ctrl+K
        if key.code == crossterm::event::KeyCode::Char('k')
            && key.modifiers.contains(crossterm::event::KeyModifiers::CONTROL)
            && let Some(cb) = self.ctrl_k.take()
        {
            cb(app);
            return Some(TransitionAction::None);
        }

        // Alt+Enter
        if key.code == crossterm::event::KeyCode::Enter
            && key.modifiers.contains(crossterm::event::KeyModifiers::ALT)
            && let Some(cb) = self.alt_enter.take()
        {
            cb(app);
            return Some(TransitionAction::None);
        }

        // Shift+Enter
        if key.code == crossterm::event::KeyCode::Enter
            && key.modifiers.contains(crossterm::event::KeyModifiers::SHIFT)
            && let Some(cb) = self.shift_enter.take()
        {
            cb(app);
            return Some(TransitionAction::None);
        }

        // Esc
        if key.code == crossterm::event::KeyCode::Esc
            && let Some(cb) = self.esc.take()
        {
            cb(app);
            return Some(TransitionAction::None);
        }

        // Enter (without Shift/Alt)
        if key.code == crossterm::event::KeyCode::Enter
            && !key.modifiers.contains(crossterm::event::KeyModifiers::SHIFT)
            && !key.modifiers.contains(crossterm::event::KeyModifiers::ALT)
            && let Some(cb) = self.enter.take()
        {
            let action = cb(app);
            return Some(action);
        }

        None
    }
}
