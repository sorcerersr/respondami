//! Tests for `SessionDisplayAdapter` — hook instruction box reconstruction (M3).

use super::SessionEntry;
use super::display_adapter::SessionDisplayAdapter;
use crate::session::AgentMessage;
use crate::tui::messages::ChatMessage;

fn user_entry(content: &str) -> SessionEntry {
    SessionEntry::new_message(
        None,
        AgentMessage::User {
            content: content.to_string(),
        },
    )
}

/// The exact frame shape the legacy framer persisted (old sessions only).
fn framed(name: &str, body: &str) -> String {
    format!("\n\n[Hook instruction — {name}]\n{body}")
}

#[test]
fn single_hook_block_renders_as_hook_box() {
    let adapter = SessionDisplayAdapter::new(false);
    let entries = vec![user_entry(&framed(
        "guard.sh",
        "add your entry to progress.md",
    ))];

    let messages = adapter.build_messages(&entries);

    assert_eq!(messages.len(), 1);
    match &messages[0] {
        ChatMessage::Hook(hook) => {
            assert_eq!(hook.hook_name, "guard.sh");
            assert_eq!(hook.stdout, "add your entry to progress.md");
            assert!(hook.success);
            assert_eq!(hook.stderr, "");
            assert!(hook.tool_name.is_none());
        }
        other => panic!("expected Hook message, got {other:?}"),
    }
}

#[test]
fn multi_hook_blocks_render_in_order() {
    let adapter = SessionDisplayAdapter::new(false);
    let content = framed("a.sh", "body A") + &framed("b.sh", "body B");
    let entries = vec![user_entry(&content)];

    let messages = adapter.build_messages(&entries);

    assert_eq!(messages.len(), 2, "one hook box per framed block");
    match &messages[0] {
        ChatMessage::Hook(hook) => assert_eq!(hook.hook_name, "a.sh"),
        other => panic!("expected Hook, got {other:?}"),
    }
    match &messages[1] {
        ChatMessage::Hook(hook) => {
            assert_eq!(hook.hook_name, "b.sh");
            assert_eq!(hook.stdout, "body B");
        }
        other => panic!("expected Hook, got {other:?}"),
    }
}

#[test]
fn plain_user_message_untouched() {
    let adapter = SessionDisplayAdapter::new(false);
    let entries = vec![user_entry("just a normal user message")];

    let messages = adapter.build_messages(&entries);

    assert_eq!(messages.len(), 1);
    match &messages[0] {
        ChatMessage::User(user) => assert_eq!(user.content, "just a normal user message"),
        other => panic!("expected User message, got {other:?}"),
    }
}

#[test]
fn malformed_framing_falls_through_to_user_bubble() {
    let adapter = SessionDisplayAdapter::new(false);

    // Frame prefix present but no closing `]` on the header line.
    let no_close = user_entry("\n\n[Hook instruction — guard.sh\nno close bracket");
    // Frame prefix present with an empty name.
    let empty_name = user_entry("\n\n[Hook instruction — ]\nbody");
    // Text before the first frame.
    let leading_text = user_entry(&format!("hello{}", framed("guard.sh", "body")));

    for entry in [no_close, empty_name, leading_text] {
        let messages = adapter.build_messages(&[entry]);
        assert_eq!(messages.len(), 1);
        match &messages[0] {
            ChatMessage::User(_) => {}
            other => panic!("expected User fallback, got {other:?}"),
        }
    }
}
