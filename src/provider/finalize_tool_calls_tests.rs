//! Tests for `finalize_tool_calls` — validated assembly of streamed tool calls.

use super::{PartialToolCall, finalize_tool_calls};

fn partial(id: &str, name: &str, arguments: &str) -> PartialToolCall {
    PartialToolCall {
        id: id.to_string(),
        name: name.to_string(),
        arguments: arguments.to_string(),
    }
}

#[test]
fn finalize_passes_valid_call_through() {
    let out = finalize_tool_calls(vec![partial("call_1", "read", r#"{"path": "src/main.rs"}"#)]);
    assert_eq!(out.len(), 1);
    assert_eq!(out[0].id, "call_1");
    assert_eq!(out[0].name, "read");
    assert_eq!(out[0].arguments, serde_json::json!({"path": "src/main.rs"}));
}

#[test]
fn finalize_keeps_valid_no_arg_call() {
    // Empty arguments are a valid no-arg call, not a malformed one.
    let out = finalize_tool_calls(vec![partial("call_1", "done", "")]);
    assert_eq!(out.len(), 1);
    assert_eq!(out[0].arguments, serde_json::json!({}));
}

#[test]
fn finalize_drops_empty_id_placeholder() {
    let out = finalize_tool_calls(vec![
        partial("call_1", "read", "{}"),
        partial("", "", ""),
        partial("call_2", "write", "{}"),
    ]);
    assert_eq!(out.len(), 2);
    assert_eq!(out[0].id, "call_1");
    assert_eq!(out[1].id, "call_2");
}

#[test]
fn finalize_drops_empty_name_placeholder() {
    // A gap placeholder carries no id or name — dropping it keeps the
    // pairing invariant (it is never executed or saved).
    let out = finalize_tool_calls(vec![partial("call_1", "", "{}")]);
    assert!(out.is_empty());
}

#[test]
fn finalize_drops_unparseable_arguments() {
    let out = finalize_tool_calls(vec![partial("call_1", "read", "not even close {{{")]);
    assert!(out.is_empty());
}

#[test]
fn finalize_preserves_order_and_mixed_drops() {
    let out = finalize_tool_calls(vec![
        partial("call_1", "read", "{}"),
        partial("", "", ""),
        partial("call_2", "bash", r#"{"command": "ls"}"#),
        partial("call_3", "", ""),
        partial("call_4", "write", "{}"),
    ]);
    let ids: Vec<&str> = out.iter().map(|tc| tc.id.as_str()).collect();
    assert_eq!(ids, ["call_1", "call_2", "call_4"]);
}
