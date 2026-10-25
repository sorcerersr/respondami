use super::welcome_screen::WelcomeScreen;
use crate::config::Config;
use crate::hooks::{Hook, HookEvent, HookSource, HookRegistry, ToolMatcher};
use crate::tui::App;
use crate::tui::theme::Theme;
use ratatui::backend::TestBackend;
use ratatui::layout::Rect;
use ratatui::style::Color;
use ratatui::Terminal;

const DIM_COLOR: Color = Color::Rgb(0x65, 0x6c, 0x76);

/// Render the welcome screen into a `TestBackend` of the given size.
///
/// `TestBackend` never touches the real terminal, so this is hermetic. The
/// point of the H-2 tests is that rendering a short terminal does not panic
/// (pre-fix, the unclamped row writes indexed past the buffer bottom).
fn render_welcome(width: u16, height: u16) {
    let backend = TestBackend::new(width, height);
    let mut terminal = Terminal::new(backend).expect("test terminal");
    let app = App::new(Config::default(), std::path::PathBuf::from("."));
    let theme = Theme::gh_dark();
    terminal
        .draw(|frame| {
            WelcomeScreen::render(frame, Rect::new(0, 0, width, height), &app, &theme);
        })
        .expect("render welcome screen");
}

#[test]
fn welcome_short_three_column_does_not_panic() {
    // 120 wide → three-column layout; 10 tall → content overflows the buffer.
    // Pre-fix, the help lines wrote past the buffer bottom and panicked.
    render_welcome(120, 10);
}

#[test]
fn welcome_short_single_column_does_not_panic() {
    // 40 wide → stacked single-column layout; 10 tall → severe overflow.
    render_welcome(40, 10);
}

#[test]
fn welcome_very_short_does_not_panic() {
    // Extreme case: only 4 rows tall. Everything below the big text is clipped.
    render_welcome(120, 4);
}

#[test]
fn welcome_normal_size_still_renders() {
    // A normal terminal must still render without panic (regression guard).
    render_welcome(120, 30);
}

fn make_hook(event: HookEvent, name: &str, source: HookSource) -> Hook {
    Hook {
        event,
        name: name.to_string(),
        command: "echo test".to_string(),
        script: None,
        source,
        skill_name: None,
        skill_dir: None,
        matcher: ToolMatcher::All,
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
