//! Unit tests for the vte-based input reader's chunk parsing and OSC 5522 capture.

use super::*;

/// Helper: create a fresh parser + handler and parse a single chunk.
fn parse(bytes: &[u8]) -> Vec<InputEvent> {
    let mut handler = InputHandler::new();
    let mut parser = Parser::new();
    parse_chunk(&mut parser, &mut handler, bytes)
}

#[test]
fn bare_escape_emits_esc() {
    let events = parse(&[0x1B]);
    assert_eq!(
        events,
        vec![InputEvent::Crossterm(Event::Key(
            crossterm::event::KeyCode::Esc.into()
        ))]
    );
}

#[test]
fn plain_char() {
    let events = parse(b"a");
    assert_eq!(
        events,
        vec![InputEvent::Crossterm(Event::Key(
            crossterm::event::KeyCode::Char('a').into()
        ))]
    );
}

#[test]
fn osc_5522_captured() {
    // OSC 5522: ESC ] 5522;type=read:status=OK:pw=<b64> ST  (b64("abc")=YWJj)
    let bytes: Vec<u8> = vec![
        0x1B, b']', b'5', b'5', b'2', b'2', b';', b't', b'y', b'p', b'e', b'=', b'r', b'e', b'a',
        b'd', b':', b's', b't', b'a', b't', b'u', b's', b'=', b'O', b'K', b':', b'p', b'w', b'=',
        b'Y', b'W', b'J', b'j', 0x1B, b'\\',
    ];
    let events = parse(&bytes);
    assert_eq!(events.len(), 1);
    match &events[0] {
        InputEvent::Kitty5522(payload) => {
            let frame = kitty5522::parse(payload).expect("should parse");
            assert_eq!(frame.status, "OK");
            assert_eq!(frame.pw.as_deref(), Some("abc"));
        }
        other => panic!("expected Kitty5522, got {other:?}"),
    }
}

#[test]
fn osc_5522_bel_terminated() {
    // An OSC 5522 terminated by BEL (0x07) instead of ST.
    let bytes: Vec<u8> = vec![
        0x1B, b']', b'5', b'5', b'2', b'2', b';', b't', b'y', b'p', b'e', b'=', b'r', b'e', b'a',
        b'd', b':', b's', b't', b'a', b't', b'u', b's', b'=', b'D', b'O', b'N', b'E', 0x07,
    ];
    let events = parse(&bytes);
    assert_eq!(events.len(), 1);
    match &events[0] {
        InputEvent::Kitty5522(payload) => {
            let frame = kitty5522::parse(payload).expect("should parse");
            assert_eq!(frame.status, "DONE");
        }
        other => panic!("expected Kitty5522, got {other:?}"),
    }
}

#[test]
fn non_5522_osc_no_kitty_event() {
    // An OSC with a different command (e.g. title, ESC ] 0 ; title ST) is not a
    // 5522 frame and must not produce a Kitty5522 event.
    let bytes: Vec<u8> = vec![0x1B, b']', b'0', b';', b't', b'i', b't', b'l', b'e', 0x1B, b'\\'];
    let events = parse(&bytes);
    assert!(
        !events.iter().any(|e| matches!(e, InputEvent::Kitty5522(_))),
        "must not produce a Kitty5522 event, got {events:?}"
    );
}

#[test]
fn mixed_chunk_key_then_osc() {
    // A key followed by a 5522 OSC in the same chunk.
    let bytes: Vec<u8> = vec![
        b'a', 0x1B, b']', b'5', b'5', b'2', b'2', b';', b't', b'y', b'p', b'e', b'=', b'r', b'e',
        b'a', b'd', b':', b's', b't', b'a', b't', b'u', b's', b'=', b'D', b'O', b'N', b'E', 0x1B,
        b'\\',
    ];
    let events = parse(&bytes);
    assert_eq!(events.len(), 2);
    assert!(matches!(&events[0], InputEvent::Crossterm(Event::Key(_))));
    assert!(matches!(&events[1], InputEvent::Kitty5522(_)));
}

#[test]
fn osc_split_across_chunks() {
    // An OSC 5522 split across two reads: the persistent parser reassembles it.
    let mut handler = InputHandler::new();
    let mut parser = Parser::new();
    let part1: Vec<u8> = vec![0x1B, b']', b'5', b'5', b'2', b'2', b';', b't', b'y', b'p', b'e'];
    let part2: Vec<u8> = vec![
        b'=', b'r', b'e', b'a', b'd', b':', b's', b't', b'a', b't', b'u', b's', b'=', b'D', b'O',
        b'N', b'E', 0x1B, b'\\',
    ];
    let events1 = parse_chunk(&mut parser, &mut handler, &part1);
    assert!(
        events1.is_empty(),
        "no complete frame yet, got {events1:?}"
    );
    let events2 = parse_chunk(&mut parser, &mut handler, &part2);
    assert_eq!(events2.len(), 1);
    match &events2[0] {
        InputEvent::Kitty5522(payload) => {
            let frame = kitty5522::parse(payload).expect("should parse");
            assert_eq!(frame.status, "DONE");
        }
        other => panic!("expected Kitty5522, got {other:?}"),
    }
}

#[test]
fn escape_sequence_split_across_chunks() {
    // An escape sequence (ESC [ A = Up arrow) split across two reads:
    // the persistent parser must reassemble it.
    let mut handler = InputHandler::new();
    let mut parser = Parser::new();
    let part1: Vec<u8> = vec![0x1B, b'['];
    let part2: Vec<u8> = vec![b'A'];
    let events1 = parse_chunk(&mut parser, &mut handler, &part1);
    assert!(
        events1.is_empty(),
        "no complete key yet, got {events1:?}"
    );
    let events2 = parse_chunk(&mut parser, &mut handler, &part2);
    assert_eq!(events2.len(), 1);
    match &events2[0] {
        InputEvent::Crossterm(Event::Key(key)) => {
            assert_eq!(key.code, crossterm::event::KeyCode::Up);
        }
        other => panic!("expected Up key, got {other:?}"),
    }
}
