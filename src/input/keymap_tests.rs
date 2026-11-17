//! Regression battery: feed raw terminal sequences through the vte-based
//! [`InputHandler`] and assert the produced `crossterm::event::Event` matches
//! what crossterm's own parser produces (hard-coded from crossterm 0.29's
//! `parse.rs`, which is `pub(crate)` and not directly callable).
//!
//! This is the load-bearing guard for the input-layer rewrite: if the vte
//! mapping diverges from crossterm's behavior, these tests fail.

use super::*;
use crossterm::event::{
    KeyCode, KeyEvent, KeyEventKind, KeyEventState, KeyModifiers, MouseButton, MouseEvent,
    MouseEventKind,
};

/// Feed raw bytes through a fresh handler and return the produced events.
fn run(bytes: &[u8]) -> Vec<InputEvent> {
    let mut handler = InputHandler::new();
    let mut parser = Parser::new();
    parse_chunk(&mut parser, &mut handler, bytes)
}

/// Assert the single produced event equals the expected `KeyEvent`.
fn assert_key(bytes: &[u8], expected: KeyEvent) {
    let events = run(bytes);
    assert_eq!(
        events.len(),
        1,
        "expected exactly 1 event for {:?}, got {events:?}",
        String::from_utf8_lossy(bytes)
    );
    match &events[0] {
        InputEvent::Crossterm(Event::Key(k)) => {
            assert_eq!(*k, expected, "for {:?}", String::from_utf8_lossy(bytes));
        }
        other => panic!("expected Key, got {other:?} for {:?}", String::from_utf8_lossy(bytes)),
    }
}

fn key(code: KeyCode) -> KeyEvent {
    code.into()
}

#[test]
fn csi_arrows() {
    assert_key(b"\x1b[A", key(KeyCode::Up));
    assert_key(b"\x1b[B", key(KeyCode::Down));
    assert_key(b"\x1b[C", key(KeyCode::Right));
    assert_key(b"\x1b[D", key(KeyCode::Left));
}

#[test]
fn ss3_arrows() {
    assert_key(b"\x1bOA", key(KeyCode::Up));
    assert_key(b"\x1bOB", key(KeyCode::Down));
    assert_key(b"\x1bOC", key(KeyCode::Right));
    assert_key(b"\x1bOD", key(KeyCode::Left));
}

#[test]
fn csi_home_end() {
    assert_key(b"\x1b[H", key(KeyCode::Home));
    assert_key(b"\x1b[F", key(KeyCode::End));
    assert_key(b"\x1bOH", key(KeyCode::Home));
    assert_key(b"\x1bOF", key(KeyCode::End));
}

#[test]
fn f_keys() {
    // SS3 F1-F4
    assert_key(b"\x1bOP", key(KeyCode::F(1)));
    assert_key(b"\x1bOQ", key(KeyCode::F(2)));
    assert_key(b"\x1bOS", key(KeyCode::F(4)));
    // CSI ~ F5-F12
    assert_key(b"\x1b[15~", key(KeyCode::F(5)));
    assert_key(b"\x1b[17~", key(KeyCode::F(6)));
    assert_key(b"\x1b[18~", key(KeyCode::F(7)));
    assert_key(b"\x1b[19~", key(KeyCode::F(8)));
    assert_key(b"\x1b[20~", key(KeyCode::F(9)));
    assert_key(b"\x1b[21~", key(KeyCode::F(10)));
    assert_key(b"\x1b[23~", key(KeyCode::F(11)));
    assert_key(b"\x1b[24~", key(KeyCode::F(12)));
}

#[test]
fn special_keys() {
    assert_key(b"\x1b[5~", key(KeyCode::PageUp));
    assert_key(b"\x1b[6~", key(KeyCode::PageDown));
    assert_key(b"\x1b[1~", key(KeyCode::Home));
    assert_key(b"\x1b[4~", key(KeyCode::End));
    assert_key(b"\x1b[2~", key(KeyCode::Insert));
    assert_key(b"\x1b[3~", key(KeyCode::Delete));
}

#[test]
fn back_tab() {
    let expected =
        KeyEvent::new_with_kind(KeyCode::BackTab, KeyModifiers::SHIFT, KeyEventKind::Press);
    assert_key(b"\x1b[Z", expected);
}

#[test]
fn focus() {
    let events = run(b"\x1b[I");
    assert_eq!(events, vec![InputEvent::Crossterm(Event::FocusGained)]);
    let events = run(b"\x1b[O");
    assert_eq!(events, vec![InputEvent::Crossterm(Event::FocusLost)]);
}

#[test]
fn enter_tab_backspace() {
    assert_key(b"\r", key(KeyCode::Enter));
    assert_key(b"\t", key(KeyCode::Tab));
    assert_key(b"\x7f", key(KeyCode::Backspace));
}

#[test]
fn ctrl_keys() {
    assert_key(b"\x01", KeyEvent::new(KeyCode::Char('a'), KeyModifiers::CONTROL));
    assert_key(b"\x1a", KeyEvent::new(KeyCode::Char('z'), KeyModifiers::CONTROL));
    assert_key(b"\x0c", KeyEvent::new(KeyCode::Char('l'), KeyModifiers::CONTROL));
    // Ctrl+4..7
    assert_key(b"\x1c", KeyEvent::new(KeyCode::Char('4'), KeyModifiers::CONTROL));
    assert_key(b"\x1f", KeyEvent::new(KeyCode::Char('7'), KeyModifiers::CONTROL));
}

#[test]
fn ctrl_space() {
    assert_key(b"\0", KeyEvent::new(KeyCode::Char(' '), KeyModifiers::CONTROL));
}

#[test]
fn bare_escape() {
    assert_key(b"\x1b", key(KeyCode::Esc));
}

#[test]
fn alt_key() {
    assert_key(b"\x1ba", KeyEvent::new(KeyCode::Char('a'), KeyModifiers::ALT));
}

#[test]
fn plain_chars() {
    assert_key(b"a", KeyEvent::new(KeyCode::Char('a'), KeyModifiers::empty()));
    // Uppercase adds SHIFT.
    assert_key(b"A", KeyEvent::new(KeyCode::Char('A'), KeyModifiers::SHIFT));
}

#[test]
fn sgr_mouse_scroll() {
    // SGR scroll up: ESC [ < 4 ; 10 ; 5 ; M
    let events = run(b"\x1b[<4;10;5M");
    assert_eq!(
        events,
        vec![InputEvent::Crossterm(Event::Mouse(MouseEvent {
            kind: MouseEventKind::ScrollUp,
            column: 9,
            row: 4,
            modifiers: KeyModifiers::empty(),
        }))]
    );
    // SGR scroll down: ESC [ < 5 ; 10 ; 5 ; M
    let events = run(b"\x1b[<5;10;5M");
    assert_eq!(
        events,
        vec![InputEvent::Crossterm(Event::Mouse(MouseEvent {
            kind: MouseEventKind::ScrollDown,
            column: 9,
            row: 4,
            modifiers: KeyModifiers::empty(),
        }))]
    );
}

#[test]
fn sgr_mouse_button() {
    // SGR left down: ESC [ < 0 ; 5 ; 3 ; M
    let events = run(b"\x1b[<0;5;3M");
    assert_eq!(
        events,
        vec![InputEvent::Crossterm(Event::Mouse(MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: 4,
            row: 2,
            modifiers: KeyModifiers::empty(),
        }))]
    );
    // SGR left up (release): ESC [ < 0 ; 5 ; 3 ; m
    let events = run(b"\x1b[<0;5;3m");
    assert_eq!(
        events,
        vec![InputEvent::Crossterm(Event::Mouse(MouseEvent {
            kind: MouseEventKind::Up(MouseButton::Left),
            column: 4,
            row: 2,
            modifiers: KeyModifiers::empty(),
        }))]
    );
}

#[test]
fn rxvt_mouse_scroll() {
    // RXVT scroll up: ESC [ 36 ; 10 ; 5 ; M  (32+4 = 36 for scroll up)
    let events = run(b"\x1b[36;10;5M");
    assert_eq!(
        events,
        vec![InputEvent::Crossterm(Event::Mouse(MouseEvent {
            kind: MouseEventKind::ScrollUp,
            column: 9,
            row: 4,
            modifiers: KeyModifiers::empty(),
        }))]
    );
}

#[test]
fn csi_u_encoded() {
    // CSI 13 u = Enter
    assert_key(b"\x1b[13u", key(KeyCode::Enter));
    // CSI 27 u = Esc
    assert_key(b"\x1b[27u", key(KeyCode::Esc));
    // CSI 97 u = 'a'
    assert_key(b"\x1b[97u", KeyEvent::new(KeyCode::Char('a'), KeyModifiers::empty()));
    // CSI 97 ; 5 u = Ctrl+a (modifier mask 5 = 1-based CTRL)
    assert_key(b"\x1b[97;5u", KeyEvent::new(KeyCode::Char('a'), KeyModifiers::CONTROL));
}

#[test]
fn csi_u_functional() {
    // CSI 57417 u = Left (keypad range → KEYPAD state)
    let expected = KeyEvent::new_with_kind_and_state(
        KeyCode::Left,
        KeyModifiers::empty(),
        KeyEventKind::Press,
        KeyEventState::KEYPAD,
    );
    assert_key(b"\x1b[57417u", expected);
    // CSI 57419 u = Up (keypad range → KEYPAD state)
    let expected = KeyEvent::new_with_kind_and_state(
        KeyCode::Up,
        KeyModifiers::empty(),
        KeyEventKind::Press,
        KeyEventState::KEYPAD,
    );
    assert_key(b"\x1b[57419u", expected);
    // CSI 57426 u = Delete (keypad range → KEYPAD state)
    let expected = KeyEvent::new_with_kind_and_state(
        KeyCode::Delete,
        KeyModifiers::empty(),
        KeyEventKind::Press,
        KeyEventState::KEYPAD,
    );
    assert_key(b"\x1b[57426u", expected);
}

#[test]
fn multi_byte_utf8_char() {
    // A UTF-8 character (é = 0xC3 0xA9)
    assert_key(b"\xc3\xa9", KeyEvent::new(KeyCode::Char('\u{e9}'), KeyModifiers::empty()));
}

#[test]
fn multi_event_chunk() {
    // Two events in one chunk: 'a' then Enter.
    let events = run(b"a\r");
    assert_eq!(
        events,
        vec![
            InputEvent::Crossterm(Event::Key(KeyEvent::new(
                KeyCode::Char('a'),
                KeyModifiers::empty()
            ))),
            InputEvent::Crossterm(Event::Key(key(KeyCode::Enter))),
        ]
    );
}

#[test]
fn unknown_csi_ignored() {
    // An unhandled CSI (e.g. cursor position `ESC [ 10 ; 20 R`) produces no Event.
    let events = run(b"\x1b[10;20R");
    assert!(
        events.is_empty(),
        "expected no events, got {events:?}"
    );
}

#[test]
fn keypad_state() {
    // CSI 57417 ; 129 u = Left, keypad (from codepoint) + NUM_LOCK (129-1=128).
    let expected = KeyEvent::new_with_kind_and_state(
        KeyCode::Left,
        KeyModifiers::empty(),
        KeyEventKind::Press,
        KeyEventState::KEYPAD | KeyEventState::NUM_LOCK,
    );
    assert_key(b"\x1b[57417;129u", expected);
}
