//! Mapping from `vte` parser callbacks to `crossterm::event::Event` values.
//!
//! This replicates the key/modifier/mouse/focus mapping in crossterm's
//! `parse.rs` so the existing `key_handler` chain is byte-for-byte unchanged.
//! The load-bearing regression guard is `keymap_tests`, which feeds the same
//! raw sequences through both crossterm's parser and this mapping and asserts
//! identical `Event` output.
//!
//! Only `Event`-producing sequences are handled. crossterm's `InternalEvent`
//! values that are not `Event`s (cursor position, keyboard-enhancement flags,
//! primary device attributes) are ignored — the app never consumes them.

use crossterm::event::{
    Event, KeyCode, KeyEvent, KeyEventKind, KeyEventState, KeyModifiers, MediaKeyCode,
    ModifierKeyCode, MouseButton, MouseEvent, MouseEventKind,
};
use vte::Params;

/// The first sub-parameter of each CSI parameter, in order.
fn param_values(params: &Params) -> Vec<u16> {
    params.iter().map(|p| p.first().copied().unwrap_or(0)).collect()
}

/// A regular printable character → `KeyCode::Char`, with `SHIFT` for uppercase
/// (matching crossterm's `char_code_to_event`). vte routes `DEL` (`0x7F`) to
/// `print` rather than `execute`, so it is handled here as Backspace.
#[must_use]
pub fn from_print(c: char) -> Option<Event> {
    if c == '\u{7f}' {
        return Some(Event::Key(KeyCode::Backspace.into()));
    }
    let modifiers = if c.is_uppercase() {
        KeyModifiers::SHIFT
    } else {
        KeyModifiers::empty()
    };
    Some(Event::Key(KeyEvent::new(KeyCode::Char(c), modifiers)))
}

/// A C0 control byte → the matching key (matching crossterm's `parse_event`).
#[must_use]
pub fn from_execute(byte: u8) -> Option<Event> {
    let key = match byte {
        b'\r' => KeyCode::Enter,
        b'\t' => KeyCode::Tab,
        b'\x7F' => KeyCode::Backspace,
        b'\0' => return Some(Event::Key(KeyEvent::new(
            KeyCode::Char(' '),
            KeyModifiers::CONTROL,
        ))),
        c @ b'\x01'..=b'\x1A' => {
            return Some(Event::Key(KeyEvent::new(
                KeyCode::Char((c - 0x1 + b'a') as char),
                KeyModifiers::CONTROL,
            )))
        }
        c @ b'\x1C'..=b'\x1F' => {
            return Some(Event::Key(KeyEvent::new(
                KeyCode::Char((c - 0x1C + b'4') as char),
                KeyModifiers::CONTROL,
            )))
        }
        _ => return None,
    };
    Some(Event::Key(key.into()))
}

/// An escape-sequence final byte (after `ESC`, not `ESC [`/`ESC ]`/`ESC P`).
///
/// `intermediates` is empty for a plain `ESC <byte>` (Alt+key) and `[b'O']` for
/// an SS3 sequence (`ESC O <byte>`, arrows/F1-F4).
#[must_use]
pub fn from_esc(intermediates: &[u8], byte: u8) -> Option<Event> {
    if intermediates == *b"O" {
        let key = match byte {
            b'D' => KeyCode::Left,
            b'C' => KeyCode::Right,
            b'A' => KeyCode::Up,
            b'B' => KeyCode::Down,
            b'H' => KeyCode::Home,
            b'F' => KeyCode::End,
            val @ b'P'..=b'S' => KeyCode::F(1 + (val - b'P')),
            _ => return None,
        };
        return Some(Event::Key(key.into()));
    }
    if intermediates.is_empty() {
        // `ESC <byte>` = Alt + the byte as a character.
        let c = byte as char;
        if c.is_ascii_graphic() || c == ' ' {
            return Some(Event::Key(KeyEvent::new(
                KeyCode::Char(c),
                KeyModifiers::ALT,
            )));
        }
    }
    None
}

/// An SS3 sequence final byte (`ESC O <byte>`, arrows / F1-F4). vte does not
/// recognize SS3 as a distinct sequence, so the `InputHandler` sets a pending
/// flag on `ESC O` and calls this with the following byte.
#[must_use]
pub fn from_ss3(byte: char) -> Option<Event> {
    let key = match byte {
        'D' => KeyCode::Left,
        'C' => KeyCode::Right,
        'A' => KeyCode::Up,
        'B' => KeyCode::Down,
        'H' => KeyCode::Home,
        'F' => KeyCode::End,
        c @ 'P'..='S' => KeyCode::F(c as u8 - b'P' + 1),
        _ => return None,
    };
    Some(Event::Key(key.into()))
}

/// A CSI sequence → the matching event (matching crossterm's `parse_csi` and its
/// helpers). `intermediates` distinguishes SGR mouse (`[b'<']`) from RXVT mouse
/// (`[]`).
#[must_use]
pub fn from_csi(params: &Params, intermediates: &[u8], action: char) -> Option<Event> {
    match action {
        'A' => Some(Event::Key(KeyCode::Up.into())),
        'B' => Some(Event::Key(KeyCode::Down.into())),
        'C' => Some(Event::Key(KeyCode::Right.into())),
        'D' => Some(Event::Key(KeyCode::Left.into())),
        'H' => Some(Event::Key(KeyCode::Home.into())),
        'F' => Some(Event::Key(KeyCode::End.into())),
        'Z' => Some(Event::Key(KeyEvent::new_with_kind(
            KeyCode::BackTab,
            KeyModifiers::SHIFT,
            KeyEventKind::Press,
        ))),
        'I' => Some(Event::FocusGained),
        'O' => Some(Event::FocusLost),
        // Legacy functional keys (Kitty keyboard protocol): `CSI 1 P` etc. with
        // no modifiers omits the leading 1.
        'P' => Some(Event::Key(KeyCode::F(1).into())),
        'Q' => Some(Event::Key(KeyCode::F(2).into())),
        'S' => Some(Event::Key(KeyCode::F(4).into())),
        '~' => from_csi_special_key(params),
        'u' => from_csi_u_encoded(params),
        'M' | 'm' => from_csi_mouse(params, intermediates, action),
        _ => None,
    }
}

/// `CSI <num>[;<mods>] ~` special keys (matching `parse_csi_special_key_code`).
fn from_csi_special_key(params: &Params) -> Option<Event> {
    let vals = param_values(params);
    let first = *vals.first()?;
    let (modifiers, kind, state) = if vals.len() > 1 {
        let mask = *vals.get(1)? as u8;
        (
            parse_modifiers(mask),
            parse_kind(mask),
            parse_modifiers_to_state(mask),
        )
    } else {
        (KeyModifiers::NONE, KeyEventKind::Press, KeyEventState::NONE)
    };
    let keycode = match first {
        1 | 7 => KeyCode::Home,
        2 => KeyCode::Insert,
        3 => KeyCode::Delete,
        4 | 8 => KeyCode::End,
        5 => KeyCode::PageUp,
        6 => KeyCode::PageDown,
        v @ 11..=15 => KeyCode::F((v - 10) as u8),
        v @ 17..=21 => KeyCode::F((v - 11) as u8),
        v @ 23..=26 => KeyCode::F((v - 12) as u8),
        v @ 28..=29 => KeyCode::F((v - 15) as u8),
        v @ 31..=34 => KeyCode::F((v - 17) as u8),
        _ => return None,
    };
    Some(Event::Key(KeyEvent::new_with_kind_and_state(
        keycode, modifiers, kind, state,
    )))
}

/// `CSI <codepoint>[:<alt>][;<mods>:<kind>] u` (Kitty / CSI-u, matching
/// `parse_csi_u_encoded_key_code`).
fn from_csi_u_encoded(params: &Params) -> Option<Event> {
    let vals = param_values(params);
    let codepoint = *vals.first()? as u32;
    let (mut modifiers, kind, state_from_modifiers) = if vals.len() > 1 {
        let mask = *vals.get(1)? as u8;
        (
            parse_modifiers(mask),
            parse_kind(mask),
            parse_modifiers_to_state(mask),
        )
    } else {
        (KeyModifiers::NONE, KeyEventKind::Press, KeyEventState::NONE)
    };

    let (keycode, state_from_keycode) = if let Some((kc, st)) = translate_functional_key(codepoint)
    {
        (kc, st)
    } else {
        let c = char::from_u32(codepoint)?;
        (
            match c {
                '\x1B' => KeyCode::Esc,
                '\r' => KeyCode::Enter,
                '\t' => {
                    if modifiers.contains(KeyModifiers::SHIFT) {
                        KeyCode::BackTab
                    } else {
                        KeyCode::Tab
                    }
                }
                '\x7F' => KeyCode::Backspace,
                _ => KeyCode::Char(c),
            },
            KeyEventState::empty(),
        )
    };

    if let KeyCode::Modifier(modifier) = keycode {
        modifiers |= modifier_to_flag(modifier);
    }

    Some(Event::Key(KeyEvent::new_with_kind_and_state(
        keycode, modifiers, kind, state_from_keycode | state_from_modifiers,
    )))
}

/// Mouse: `CSI < Cb ; Cx ; Cy ; M|m` (SGR, 0-based Cb) or `CSI Cb ; Cx ; Cy ; M`
/// (RXVT, 32-based Cb). SGR uses a lowercase `m` for button release.
fn from_csi_mouse(params: &Params, intermediates: &[u8], action: char) -> Option<Event> {
    let vals = param_values(params);
    let cb = vals.first().copied()?;
    let cx = vals.get(1).copied()?;
    let cy = vals.get(2).copied()?;
    let is_sgr = intermediates == *b"<";
    let cb = if is_sgr { cb } else { cb.checked_sub(32)? } as u8;
    let (mut kind, modifiers) = parse_cb(cb)?;
    // SGR release: lowercase `m` converts a press to a release.
    if is_sgr && action == 'm'
        && let MouseEventKind::Down(button) = kind {
            kind = MouseEventKind::Up(button);
        }
    Some(Event::Mouse(MouseEvent {
        kind,
        column: cx.saturating_sub(1),
        row: cy.saturating_sub(1),
        modifiers,
    }))
}

/// `Cb` mouse button/modifier byte.
///
/// Uses the standard X11/SGR layout: the button is the low 3 bits (0-7) and the
/// modifiers are bits 2-4 (shift=4, meta=8, ctrl=16). This is a deliberate fix
/// over crossterm's `parse_cb`, which only uses the low 2 bits for the button
/// and therefore misreads scroll (Cb 4-7) as a button press + SHIFT. The app
/// relies on `ScrollUp`/`ScrollDown`, so the correct layout is required.
fn parse_cb(cb: u8) -> Option<(MouseEventKind, KeyModifiers)> {
    let button_number = cb & 0b111;
    // The X11 layout overlaps bit 2 between the scroll buttons (4-7) and the
    // shift modifier. For scroll, bit 2 is part of the button; for the main
    // buttons (0-3) it is shift.
    let is_scroll = (4..=7).contains(&button_number);
    let dragging = cb & 0b10000 == 0b10000;
    let kind = match (button_number, dragging) {
        (0, false) => MouseEventKind::Down(MouseButton::Left),
        (1, false) => MouseEventKind::Down(MouseButton::Middle),
        (2, false) => MouseEventKind::Down(MouseButton::Right),
        (0, true) => MouseEventKind::Drag(MouseButton::Left),
        (1, true) => MouseEventKind::Drag(MouseButton::Middle),
        (2, true) => MouseEventKind::Drag(MouseButton::Right),
        (3, false) => MouseEventKind::Up(MouseButton::Left),
        (3..=5, true) => MouseEventKind::Moved,
        (4, false) => MouseEventKind::ScrollUp,
        (5, false) => MouseEventKind::ScrollDown,
        (6, false) => MouseEventKind::ScrollLeft,
        (7, false) => MouseEventKind::ScrollRight,
        _ => return None,
    };
    let mut modifiers = KeyModifiers::empty();
    if !is_scroll && cb & 0b100 == 0b100 {
        modifiers |= KeyModifiers::SHIFT;
    }
    if cb & 0b1000 == 0b1000 {
        modifiers |= KeyModifiers::ALT;
    }
    if cb & 0b10000 == 0b10000 {
        modifiers |= KeyModifiers::CONTROL;
    }
    Some((kind, modifiers))
}

/// 1-based modifier mask → `KeyModifiers` (matching crossterm's `parse_modifiers`).
fn parse_modifiers(mask: u8) -> KeyModifiers {
    let mask = mask.saturating_sub(1);
    let mut modifiers = KeyModifiers::empty();
    if mask & 1 != 0 {
        modifiers |= KeyModifiers::SHIFT;
    }
    if mask & 2 != 0 {
        modifiers |= KeyModifiers::ALT;
    }
    if mask & 4 != 0 {
        modifiers |= KeyModifiers::CONTROL;
    }
    if mask & 8 != 0 {
        modifiers |= KeyModifiers::SUPER;
    }
    if mask & 16 != 0 {
        modifiers |= KeyModifiers::HYPER;
    }
    if mask & 32 != 0 {
        modifiers |= KeyModifiers::META;
    }
    modifiers
}

/// 1-based modifier mask → `KeyEventState` (matching `parse_modifiers_to_state`).
fn parse_modifiers_to_state(mask: u8) -> KeyEventState {
    let mask = mask.saturating_sub(1);
    let mut state = KeyEventState::empty();
    if mask & 64 != 0 {
        state |= KeyEventState::CAPS_LOCK;
    }
    if mask & 128 != 0 {
        state |= KeyEventState::NUM_LOCK;
    }
    state
}

/// Event-type code → `KeyEventKind` (matching `parse_key_event_kind`).
fn parse_kind(kind: u8) -> KeyEventKind {
    match kind {
        1 => KeyEventKind::Press,
        2 => KeyEventKind::Repeat,
        3 => KeyEventKind::Release,
        _ => KeyEventKind::Press,
    }
}

/// A `KeyCode::Modifier` → its `KeyModifiers` flag.
fn modifier_to_flag(modifier: ModifierKeyCode) -> KeyModifiers {
    match modifier {
        ModifierKeyCode::LeftAlt | ModifierKeyCode::RightAlt => KeyModifiers::ALT,
        ModifierKeyCode::LeftControl | ModifierKeyCode::RightControl => KeyModifiers::CONTROL,
        ModifierKeyCode::LeftShift | ModifierKeyCode::RightShift => KeyModifiers::SHIFT,
        ModifierKeyCode::LeftSuper | ModifierKeyCode::RightSuper => KeyModifiers::SUPER,
        ModifierKeyCode::LeftHyper | ModifierKeyCode::RightHyper => KeyModifiers::HYPER,
        ModifierKeyCode::LeftMeta | ModifierKeyCode::RightMeta => KeyModifiers::META,
        _ => KeyModifiers::empty(),
    }
}

/// Translate a CSI-u codepoint to a special key (matching crossterm's
/// `translate_functional_key_code`).
fn translate_functional_key(codepoint: u32) -> Option<(KeyCode, KeyEventState)> {
    let keypad = KeyEventState::KEYPAD;
    let key = match codepoint {
        57399 => KeyCode::Char('0'),
        57400 => KeyCode::Char('1'),
        57401 => KeyCode::Char('2'),
        57402 => KeyCode::Char('3'),
        57403 => KeyCode::Char('4'),
        57404 => KeyCode::Char('5'),
        57405 => KeyCode::Char('6'),
        57406 => KeyCode::Char('7'),
        57407 => KeyCode::Char('8'),
        57408 => KeyCode::Char('9'),
        57409 => KeyCode::Char('.'),
        57410 => KeyCode::Char('/'),
        57411 => KeyCode::Char('*'),
        57412 => KeyCode::Char('-'),
        57413 => KeyCode::Char('+'),
        57414 => KeyCode::Enter,
        57415 => KeyCode::Char('='),
        57416 => KeyCode::Char(','),
        57417 => KeyCode::Left,
        57418 => KeyCode::Right,
        57419 => KeyCode::Up,
        57420 => KeyCode::Down,
        57421 => KeyCode::PageUp,
        57422 => KeyCode::PageDown,
        57423 => KeyCode::Home,
        57424 => KeyCode::End,
        57425 => KeyCode::Insert,
        57426 => KeyCode::Delete,
        57427 => KeyCode::KeypadBegin,
        57358 => KeyCode::CapsLock,
        57359 => KeyCode::ScrollLock,
        57360 => KeyCode::NumLock,
        57361 => KeyCode::PrintScreen,
        57362 => KeyCode::Pause,
        57363 => KeyCode::Menu,
        57376 => KeyCode::F(13),
        57377 => KeyCode::F(14),
        57378 => KeyCode::F(15),
        57379 => KeyCode::F(16),
        57380 => KeyCode::F(17),
        57381 => KeyCode::F(18),
        57382 => KeyCode::F(19),
        57383 => KeyCode::F(20),
        57384 => KeyCode::F(21),
        57385 => KeyCode::F(22),
        57386 => KeyCode::F(23),
        57387 => KeyCode::F(24),
        57388 => KeyCode::F(25),
        57389 => KeyCode::F(26),
        57390 => KeyCode::F(27),
        57391 => KeyCode::F(28),
        57392 => KeyCode::F(29),
        57393 => KeyCode::F(30),
        57394 => KeyCode::F(31),
        57395 => KeyCode::F(32),
        57396 => KeyCode::F(33),
        57397 => KeyCode::F(34),
        57398 => KeyCode::F(35),
        57428 => KeyCode::Media(MediaKeyCode::Play),
        57429 => KeyCode::Media(MediaKeyCode::Pause),
        57430 => KeyCode::Media(MediaKeyCode::PlayPause),
        57431 => KeyCode::Media(MediaKeyCode::Reverse),
        57432 => KeyCode::Media(MediaKeyCode::Stop),
        57433 => KeyCode::Media(MediaKeyCode::FastForward),
        57434 => KeyCode::Media(MediaKeyCode::Rewind),
        57435 => KeyCode::Media(MediaKeyCode::TrackNext),
        57436 => KeyCode::Media(MediaKeyCode::TrackPrevious),
        57437 => KeyCode::Media(MediaKeyCode::Record),
        57438 => KeyCode::Media(MediaKeyCode::LowerVolume),
        57439 => KeyCode::Media(MediaKeyCode::RaiseVolume),
        57440 => KeyCode::Media(MediaKeyCode::MuteVolume),
        57441 => KeyCode::Modifier(ModifierKeyCode::LeftShift),
        57442 => KeyCode::Modifier(ModifierKeyCode::LeftControl),
        57443 => KeyCode::Modifier(ModifierKeyCode::LeftAlt),
        57444 => KeyCode::Modifier(ModifierKeyCode::LeftSuper),
        57445 => KeyCode::Modifier(ModifierKeyCode::LeftHyper),
        57446 => KeyCode::Modifier(ModifierKeyCode::LeftMeta),
        57447 => KeyCode::Modifier(ModifierKeyCode::RightShift),
        57448 => KeyCode::Modifier(ModifierKeyCode::RightControl),
        57449 => KeyCode::Modifier(ModifierKeyCode::RightAlt),
        57450 => KeyCode::Modifier(ModifierKeyCode::RightSuper),
        57451 => KeyCode::Modifier(ModifierKeyCode::RightHyper),
        57452 => KeyCode::Modifier(ModifierKeyCode::RightMeta),
        57453 => KeyCode::Modifier(ModifierKeyCode::IsoLevel3Shift),
        57454 => KeyCode::Modifier(ModifierKeyCode::IsoLevel5Shift),
        _ => return None,
    };
    let state = if (57399..=57427).contains(&codepoint) {
        keypad
    } else {
        KeyEventState::empty()
    };
    Some((key, state))
}
