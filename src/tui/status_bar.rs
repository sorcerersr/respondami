//! Status bar rendering — activity indicator, context usage, and token stats.
//!
//! Renders the bottom status bar with animated sweep indicator, CWD, git branch,
//! model name, context window percentage (gradient-colored), and cumulative
//! token counts. Includes fallback token estimation during streaming.

use ratatui::layout::Rect;
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Clear, Paragraph};
use ratatui::Frame;
use tachyonfx::{EffectTimer, Interpolation, Motion, fx};

use super::app::App;
use super::mode::AppState;
use super::theme::Theme;
use crate::agent::token_estimation::{estimate_tokens_from_messages, estimate_tokens_from_streamed_text};
use crate::session::AgentMessage;

/// Git branch detection result.
enum GitBranch {
    /// Successfully detected a branch name.
    Ok(String),
    /// Git command failed (not a repo, permission denied, etc.).
    Error,
}

/// Detect the current git branch name.
///
/// Returns `Some(GitBranch::Ok(branch))` if inside a git working copy.
/// Returns `Some(GitBranch::Error)` if the git command fails.
/// Returns `None` if `git` is not found or the output is empty (treat as not a git repo).
fn get_git_branch() -> Option<GitBranch> {
    let output = std::process::Command::new("git")
        .args(["rev-parse", "--abbrev-ref", "HEAD"])
        .output();

    match output {
        Ok(out) if out.status.success() => {
            let branch = String::from_utf8_lossy(&out.stdout).trim().to_string();
            if branch.is_empty() {
                None
            } else {
                Some(GitBranch::Ok(branch))
            }
        }
        Ok(_) => Some(GitBranch::Error), // git ran but failed
        Err(_) => None, // git not found
    }
}

/// Green stop: 0% usage — taken from theme.green.
pub(crate) const GRADIENT_GREEN: (u8, u8, u8) = (0x57, 0xab, 0x5a);

/// Yellow mid-stop: 50% usage — computed, not in Theme.
pub(crate) const GRADIENT_YELLOW: (u8, u8, u8) = (0xe6, 0xc6, 0x2b);

/// Red stop: 100% usage — taken from theme.red.
pub(crate) const GRADIENT_RED: (u8, u8, u8) = (0xe5, 0x53, 0x4b);

/// Linearly interpolate between two RGB colors.
pub(crate) fn interpolate_rgb(c1: (u8, u8, u8), c2: (u8, u8, u8), t: f64) -> Color {
    let t = t.clamp(0.0, 1.0);
    Color::Rgb(
        (f64::from(c1.0) + (f64::from(c2.0) - f64::from(c1.0)) * t) as u8,
        (f64::from(c1.1) + (f64::from(c2.1) - f64::from(c1.1)) * t) as u8,
        (f64::from(c1.2) + (f64::from(c2.2) - f64::from(c1.2)) * t) as u8,
    )
}

/// Green(0%) → Yellow(50%) → Red(100%) gradient.
///
/// `ratio` should be in `[0.0, 1.0]`. Values outside are clamped.
/// Returns `theme.accent` if ratio is NaN.
pub(crate) fn gradient_color(ratio: f64, theme: &Theme) -> Color {
    if ratio.is_nan() {
        return theme.accent;
    }
    let t = ratio.clamp(0.0, 1.0);
    if t < 0.5 {
        interpolate_rgb(GRADIENT_GREEN, GRADIENT_YELLOW, t / 0.5)
    } else {
        interpolate_rgb(GRADIENT_YELLOW, GRADIENT_RED, (t - 0.5) / 0.5)
    }
}

/// Format a token count as a human-readable k value (e.g. 131072 → "131.1k").
pub(crate) fn format_k(value: u32) -> String {
    if value >= 1000 {
        format!("{:.1}k", f64::from(value) / 1000.0)
    } else {
        format!("{value}")
    }
}

/// Render the status bar with gradient-colored context/token segments.
///
/// Segment layout:
/// ```text
///  ◐ Streaming ◐ │ ⌂ cwd │ ⎇ branch │ model │ 12% / 131.1k │ ↑108.1k ↓22.3k
///          sweep         accent      accent      gradient       accent
/// ```
///
/// - Activity indicator: animated sweep for Streaming/ToolExec/Compacting, static for Idle
/// - Percentage: input tokens / context window (current context usage)
/// - ↑: cumulative input tokens (total prompt tokens across all requests)
/// - ↓: cumulative output tokens (total completion tokens across all requests)
///
/// Input tokens represent context sent to the model; output tokens are generated
/// by the model. Together they can exceed the context window in multi-iteration
/// turns (tool calls), which is normal — each API call independently counts tokens.
///
/// Get the activity label, color, and whether it should animate.
///
/// All client-side states (Idle, `SessionSelect`, `InitPopup`, `CommandPalette`) map to "Ready" since the LLM is idle.
#[must_use]
pub fn get_activity(state: &AppState, theme: &Theme) -> (&'static str, Color, bool) {
    match state {
        AppState::Streaming     => ("◐ Streaming ◐", theme.blue, true),
        AppState::ToolExec      => ("◆ Tool Exec ◆", theme.orange, true),
        AppState::Compacting    => ("◎ Compacting ◎", theme.purple, true),
        _ => ("● Ready ●", theme.green, false),
    }
}
pub fn render_status_bar(frame: &mut Frame, area: Rect, app: &mut App, theme: &Theme) {
    frame.render_widget(Clear, area);

    // Check for transient status bar message
    if let Some((msg, started_at)) = app.ui.status_bar_message.take() {
        let elapsed = started_at.elapsed().as_secs_f64();
        if elapsed >= 5.0 {
            // Apply slide-out effect, message expires
            let slide = fx::slide_out(
                Motion::LeftToRight,
                10,
                0,
                theme.bg,
                EffectTimer::from_ms(400, Interpolation::CubicOut),
            )
            .with_area(area);
            app.ui.effect_manager.add_effect(slide);
            // status_bar_message already taken (None)
        } else {
            // Still showing — put it back
            app.ui.status_bar_message = Some((msg.clone(), started_at));
        }

        // Render the transient message
        let msg_line = Line::from(Span::styled(
            msg,
            Style::default().fg(theme.text_muted),
        ));
        let paragraph = Paragraph::new(msg_line)
            .style(Style::default().bg(theme.bg));
        frame.render_widget(paragraph, area);
        return;
    }

    let cwd = std::env::current_dir()
        .ok()
        .and_then(|p| p.file_name().map(|n| n.to_string_lossy().to_string()))
        .unwrap_or_else(|| ".".to_string());

    let model = &app.config.model_name;
    let context_window = app.config.context_window;

    // Context usage: use session-level token totals (prompt + completion) for percentage.
    // This represents actual context window usage across the entire session.
    // During streaming, use fallback estimation to avoid the "?" gap.
    let input_tokens = app.session.current_request_usage.input_tokens;
    let has_usage = input_tokens > 0;
    let is_streaming = app.agent.tracker.is_streaming();

    // When streaming and no real usage yet, use fallback estimation.
    // Set the `estimated` flag so the status bar can show "~" prefix.
    let (estimated_input, _estimated_output) = if !has_usage && is_streaming {
        // Build full message list for input estimation:
        // 1. System prompt (via build_system_prompt_with_agents_md)
        // 2. Conversation history from session_store
        // 3. Current user message
        let (system_prompt, _) =
            crate::agent::build_system_prompt_with_agents_md(&app.config.cwd, &app.config.skills);
        let mut messages: Vec<AgentMessage> = vec![AgentMessage::system(system_prompt)];
        messages.extend(app.session.session_store.build_context());
        if let Some(user_msg) = &app.agent.current_user_message {
            messages.push(AgentMessage::user(user_msg.clone()));
        }
        let est_input = estimate_tokens_from_messages(&messages);
        let est_output = estimate_tokens_from_streamed_text(&app.agent.streaming_content);
        (est_input + est_output, est_output)
    } else {
        (0, 0)
    };

    // Set the estimated flag for the status bar indicator.
    // This is set when we're using fallback estimation, and cleared when real Usage arrives.
    let is_estimated = !has_usage && is_streaming;
    app.session.current_request_usage.estimated = is_estimated;

    // Use current prompt tokens for percentage (actual context window usage).
    // When estimated, use the estimated input; otherwise use current_request_usage.
    let total_display_tokens = if is_estimated {
        estimated_input
    } else {
        app.session.current_request_usage.input_tokens
    };
    let pct_str = if context_window > 0 {
        let pct = f64::from(total_display_tokens) / f64::from(context_window) * 100.0;
        let prefix = if is_estimated { "~" } else { "" };
        format!("{}{}%", prefix, pct as u64)
    } else {
        "0%".to_string()
    };
    let context_window_str = format_k(context_window);

    // Cumulative session totals: input (↑) and output (↓) tokens.
    // These sum across all requests and can exceed context window in
    // multi-iteration turns (tool calls) — this is normal.
    let cumulative_input = app.session.cumulative_usage.input_tokens;
    let cumulative_output = app.session.cumulative_usage.output_tokens;

    // Gradient color for context %
    let context_color = if is_estimated {
        theme.text_muted
    } else if context_window > 0 {
        let pct = f64::from(total_display_tokens) / f64::from(context_window);
        gradient_color(pct, theme)
    } else {
        theme.accent
    };

    // Detect git branch
    let git_branch = get_git_branch();

    // Activity indicator: animated sweep for working states, static for idle
    let (activity_label, activity_color, is_active) = get_activity(&app.modal.state, theme);
    let activity_spans = if is_active {
        app.ui.activity_indicator.render_spans(activity_label, activity_color, theme)
    } else {
        app.ui.activity_indicator.render_static(activity_label, activity_color, theme)
    };

    // Build status line segments
    let mut spans: Vec<Span<'_>> = vec![Span::styled(
        " ",
        Style::default().fg(theme.text).bg(theme.bg),
    )];
    spans.extend(activity_spans);

    // Separator after activity indicator
    spans.push(Span::styled(
        " │ ",
        Style::default().fg(theme.accent),
    ));
    spans.push(Span::styled(
        format!("⌂ {cwd} │"),
        Style::default().fg(theme.accent),
    ));
    match git_branch {
        Some(GitBranch::Ok(branch)) => {
            spans.push(Span::styled(
                format!(" ⎇ {branch} │"),
                Style::default().fg(theme.accent),
            ));
        }
        Some(GitBranch::Error) => {
            spans.push(Span::styled(" ⎇? │", Style::default().fg(theme.text_muted)));
        }
        None => {}
    }

    spans.push(Span::styled(
        format!(" {model} │"),
        Style::default().fg(theme.accent),
    ));
    spans.push(Span::styled(
        format!(" {pct_str} / {context_window_str}"),
        Style::default().fg(context_color),
    ));
    spans.push(Span::styled(" │ ", Style::default().fg(theme.accent)));
    spans.push(Span::styled(
        format!("↑{} ↓{}", format_k(cumulative_input), format_k(cumulative_output)),
        Style::default().fg(theme.accent),
    ));

    let status_line = Line::from(spans);

    let paragraph = Paragraph::new(status_line)
        .style(Style::default().bg(theme.bg));
    frame.render_widget(paragraph, area);
}
