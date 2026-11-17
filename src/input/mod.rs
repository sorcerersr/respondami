//! Terminal input reading via a [`vte`] parser.
//!
//! crossterm's event reader mangles the OSC sequences used by the Kitty 5522
//! clipboard protocol (it has no raw pass-through), so this module replaces
//! crossterm's event *reading* with a `vte::Parser`. The parser's callbacks are
//! mapped back to the same `crossterm::event::Event` values the rest of the app
//! already consumes, so the `key_handler` chain is unchanged. OSC 5522 frames
//! are surfaced as [`InputEvent::Kitty5522`] for the state machine in
//! [`kitty5522`].
//!
//! crossterm is still used for raw-mode, output, and as the `Event` type.
//! ratatui is render-only and is unaffected.
//!
//! # Concurrency
//!
//! [`spawn_reader`] runs a tokio task that reads stdin in chunks, feeds each
//! chunk to a persistent `vte::Parser`, and sends the resulting
//! [`InputEvent`]s over an `mpsc` channel. The `Parser` is stateful and lives
//! in that task; no state crosses threads except the channel.

pub mod kitty5522;
pub mod keymap;

use std::io::Read;

use crossterm::event::Event;
use tokio::sync::mpsc;
use vte::{Parser, Perform};

/// An input event produced by the vte-based reader.
///
/// [`Crossterm`] carries the normal key/mouse/resize/focus/paste events the app
/// already handles. [`Kitty5522`] carries a raw 5522 OSC payload for the
/// clipboard state machine.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InputEvent {
    /// A normal terminal event, identical in shape to crossterm's.
    Crossterm(Event),
    /// A raw Kitty 5522 OSC payload (the bytes after `ESC ] 5522` up to the
    /// terminating ST), for [`kitty5522`] to parse.
    Kitty5522(Vec<u8>),
}

/// A [`vte::Perform`] handler that accumulates [`InputEvent`]s.
///
/// Created fresh for each reader task; drained after each `advance` call.
#[derive(Debug, Default)]
pub struct InputHandler {
    events: Vec<InputEvent>,
    /// True after seeing `ESC O` (SS3 prefix); the next printable byte is the SS3 key.
    ss3_pending: bool,
}

impl InputHandler {
    /// Create an empty handler.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Take out and clear the accumulated events.
    pub fn drain(&mut self) -> Vec<InputEvent> {
        std::mem::take(&mut self.events)
    }
}

impl Perform for InputHandler {
    fn print(&mut self, c: char) {
        if self.ss3_pending {
            self.ss3_pending = false;
            if let Some(ev) = keymap::from_ss3(c) {
                self.events.push(InputEvent::Crossterm(ev));
            }
            return;
        }
        if let Some(ev) = keymap::from_print(c) {
            self.events.push(InputEvent::Crossterm(ev));
        }
    }

    fn execute(&mut self, byte: u8) {
        if let Some(ev) = keymap::from_execute(byte) {
            self.events.push(InputEvent::Crossterm(ev));
        }
    }

    fn csi_dispatch(
        &mut self,
        params: &vte::Params,
        intermediates: &[u8],
        _ignore: bool,
        action: char,
    ) {
        if let Some(ev) = keymap::from_csi(params, intermediates, action) {
            self.events.push(InputEvent::Crossterm(ev));
        }
    }

    fn esc_dispatch(&mut self, intermediates: &[u8], _ignore: bool, byte: u8) {
        // `ESC O` is the SS3 prefix; vte dispatches it as a plain escape with
        // final byte 'O'. Capture it and wait for the real key byte.
        if intermediates.is_empty() && byte == b'O' {
            self.ss3_pending = true;
            return;
        }
        // `ESC \\` is the ST (string terminator) that closes an OSC/DCS. It is
        // not a key; ignore it so it is not misread as Alt+backslash.
        if intermediates.is_empty() && byte == b'\\' {
            return;
        }
        if let Some(ev) = keymap::from_esc(intermediates, byte) {
            self.events.push(InputEvent::Crossterm(ev));
        }
    }

    fn osc_dispatch(&mut self, params: &[&[u8]], _bell_terminated: bool) {
        // The 5522 clipboard protocol arrives as OSC 5522. vte splits the
        // payload on `;`, so `params[0]` is `5522` and `params[1..]` is the
        // rest (metadata;payload). Re-join the rest and surface it for the
        // state machine.
        if params.first().copied() == Some(b"5522") {
            let mut payload = Vec::new();
            for part in &params[1..] {
                if !payload.is_empty() {
                    payload.push(b';');
                }
                payload.extend_from_slice(part);
            }
            self.events.push(InputEvent::Kitty5522(payload));
        }
        // All other OSC sequences (title, colors, etc.) are ignored.
    }
}

/// Feed a byte chunk to the handler and return the produced events.
///
/// A chunk that is exactly a bare `ESC` (`0x1B`) yields the Escape key, matching
/// crossterm's per-keystroke behavior. The `parser` is persistent across chunks
/// so that multi-byte escape sequences split across `read()` boundaries are
/// reassembled.
pub(crate) fn parse_chunk(
    parser: &mut Parser,
    handler: &mut InputHandler,
    bytes: &[u8],
) -> Vec<InputEvent> {
    if bytes == [0x1B] {
        return vec![InputEvent::Crossterm(Event::Key(
            crossterm::event::KeyCode::Esc.into(),
        ))];
    }
    parser.advance(handler, bytes);
    handler.drain()
}

/// Spawn a task that reads stdin and sends [`InputEvent`]s over `tx`.
///
/// The read loop runs on a single blocking thread (so the async runtime is not
/// blocked); the `Stdin` lock is held for the task's lifetime. The `Parser` is
/// persistent across reads. The returned [`tokio::task::JoinHandle`] resolves
/// when stdin closes.
#[must_use]
pub fn spawn_reader(
    tx: mpsc::Sender<InputEvent>,
) -> tokio::task::JoinHandle<()> {
    tokio::task::spawn_blocking(move || {
        let stdin = std::io::stdin();
        let mut guard = stdin.lock();
        let mut handler = InputHandler::new();
        let mut parser = Parser::new();
        let mut buf = [0u8; 4096];
        'read: loop {
            match guard.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => {
                    let events = parse_chunk(&mut parser, &mut handler, &buf[..n]);
                    for ev in events {
                        if tx.blocking_send(ev).is_err() {
                            break 'read;
                        }
                    }
                }
                Err(_) => break,
            }
        }
    })
}

#[cfg(test)]
mod input_tests;

#[cfg(test)]
mod keymap_tests;

#[cfg(test)]
mod kitty5522_tests;
