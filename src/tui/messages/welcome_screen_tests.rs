use super::welcome_screen::WelcomeScreen;
use crate::hooks::{Hook, HookEvent, HookSource, HookRegistry};
use ratatui::style::Color;

const DIM_COLOR: Color = Color::Rgb(0x65, 0x6c, 0x76);

fn make_hook(event: HookEvent, name: &str, source: HookSource) -> Hook {
    Hook {
        event,
        name: name.to_string(),
        command: "echo test".to_string(),
        source,
    }
}

#[test]
fn empty_registry_shows_no_hooks_configured() {
    let registry = HookRegistry::new();
    let lines = WelcomeScreen::build_hooks_content(&registry, DIM_COLOR);
    assert_eq!(lines.len(), 1);
    assert_eq!(lines[0].spans[0].content.as_ref(), "No hooks configured");
}

#[test]
fn single_event_type_shows_one_group() {
    let mut registry = HookRegistry::new();
    registry.add(make_hook(
        HookEvent::PreToolUse,
        "security-check.sh",
        HookSource::Global,
    ));
    let lines = WelcomeScreen::build_hooks_content(&registry, DIM_COLOR);
    assert_eq!(lines.len(), 2);
    assert_eq!(lines[0].spans[0].content.as_ref(), "PreToolUse:");
    assert_eq!(lines[1].spans[0].content.as_ref(), "  security-check.sh");
}

#[test]
fn multiple_event_types_shows_all_groups() {
    let mut registry = HookRegistry::new();
    registry.add(make_hook(
        HookEvent::PreToolUse,
        "check.sh",
        HookSource::Project,
    ));
    registry.add(make_hook(
        HookEvent::PostToolUse,
        "log.sh",
        HookSource::Global,
    ));
    let lines = WelcomeScreen::build_hooks_content(&registry, DIM_COLOR);
    assert_eq!(lines.len(), 4);
    assert_eq!(lines[0].spans[0].content.as_ref(), "PreToolUse:");
    assert_eq!(lines[1].spans[0].content.as_ref(), "  check.sh");
    assert_eq!(lines[2].spans[0].content.as_ref(), "PostToolUse:");
    assert_eq!(lines[3].spans[0].content.as_ref(), "  log.sh");
}

#[test]
fn mixed_sources_includes_both_global_and_project() {
    let mut registry = HookRegistry::new();
    registry.add(make_hook(
        HookEvent::Stop,
        "global-stop.sh",
        HookSource::Global,
    ));
    registry.add(make_hook(
        HookEvent::Stop,
        "project-stop.sh",
        HookSource::Project,
    ));
    let lines = WelcomeScreen::build_hooks_content(&registry, DIM_COLOR);
    assert_eq!(lines.len(), 3);
    assert_eq!(lines[0].spans[0].content.as_ref(), "Stop:");
    assert_eq!(lines[1].spans[0].content.as_ref(), "  global-stop.sh");
    assert_eq!(lines[2].spans[0].content.as_ref(), "  project-stop.sh");
}

#[test]
fn empty_event_types_are_skipped() {
    let mut registry = HookRegistry::new();
    registry.add(make_hook(
        HookEvent::PostToolUse,
        "only-hook.sh",
        HookSource::Project,
    ));
    // PreToolUse, Stop, PreCompact, UserPromptSubmit are all empty
    let lines = WelcomeScreen::build_hooks_content(&registry, DIM_COLOR);
    assert_eq!(lines.len(), 2);
    assert_eq!(lines[0].spans[0].content.as_ref(), "PostToolUse:");
    assert_eq!(lines[1].spans[0].content.as_ref(), "  only-hook.sh");
}
