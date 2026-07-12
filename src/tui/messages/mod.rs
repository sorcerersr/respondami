//! Chat message types and rendering.
//!
//! Defines `ChatMessage` enum and `ChatRenderer` for the chat viewport.
//! Submodules handle individual message variants (user, assistant, tool call, etc.).
//!
//! Rust guideline compliant 2026-02-21

use super::app::App;
use ratatui_md::HeightAware;
use super::hook_display::HookDisplay;
use super::thinking_display::ThinkingDisplay;
use super::theme::Theme;
use welcome_screen::WelcomeScreen;
use tool_call::ANIMATION_DURATION;
use std::time::Instant;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::Text;
use ratatui::widgets::Paragraph;
use ratatui::Frame;
use tachyonfx::{EffectTimer, fx, Interpolation};

// Submodules
mod assistant_message;
mod compaction_message;
mod hook_message;
mod skill_activation_message;
mod system_message;
mod thinking_message;
pub mod tool_call;
mod user_message;
mod welcome_screen;

#[cfg(test)]
mod hook_message_tests;
#[cfg(test)]
mod system_message_tests;
#[cfg(test)]
mod thinking_message_tests;
#[cfg(test)]
mod welcome_screen_tests;

#[doc(inline)]
pub use assistant_message::AssistantMessage;
#[doc(inline)]
pub use compaction_message::CompactionMessage;
#[doc(inline)]
pub use hook_message::HookMessage;
#[doc(inline)]
pub use skill_activation_message::SkillActivationMessage;
#[doc(inline)]
pub use system_message::SystemMessage;
#[doc(inline)]
pub use thinking_message::ThinkingMessage;
#[doc(inline)]
pub use tool_call::ToolCallMessage;
#[doc(inline)]
pub use tool_call::ToolCallVariant;

#[doc(inline)]
pub use user_message::UserMessage;

/// A chat message. Each variant owns its specific data.
#[derive(Debug, Clone)]
pub enum ChatMessage {
    User(UserMessage),
    Assistant(AssistantMessage),
    ToolCall(ToolCallMessage),
    Thinking(ThinkingMessage),
    Compaction(CompactionMessage),
    System(SystemMessage),
    Hook(HookMessage),
    SkillActivation(SkillActivationMessage),
}

impl ChatMessage {
    /// Compute height. `display` controls `ThinkingMessage` height, `hook_display` controls `HookMessage` height.
    #[must_use]
    pub fn height(&self, width: usize, display: ThinkingDisplay, max_lines: usize, hook_display: HookDisplay, theme: &Theme) -> usize {
        match self {
            ChatMessage::User(msg) => msg.height(width, theme),
            ChatMessage::Assistant(msg) => msg.height(width, theme),
            ChatMessage::ToolCall(msg) => msg.height(width, theme),
            ChatMessage::Thinking(msg) => msg.height_with(width, display, max_lines, theme),
            ChatMessage::Compaction(msg) => msg.height(width, theme),
            ChatMessage::System(msg) => msg.height(width, theme),
            ChatMessage::Hook(msg) => msg.height(width, hook_display, theme),
            ChatMessage::SkillActivation(msg) => msg.height(width, theme),
        }
    }

    /// Render this message into the given area using the buffer.
    /// Messages render in full (no clipping — `ScrollView` handles viewport).
    #[expect(clippy::too_many_arguments, reason = "render signature unified across all message types")]
    pub fn render_into(&self, area: Rect, display: ThinkingDisplay, max_lines: usize, hook_display: HookDisplay, buf: &mut ratatui::buffer::Buffer, theme: &Theme, rtk_active: bool) {
        match self {
            ChatMessage::User(msg) => msg.render_into(area, buf, theme),
            ChatMessage::Assistant(msg) => msg.render_into(area, buf, theme),
            ChatMessage::ToolCall(msg) => msg.render_into(area, buf, theme, rtk_active),
            ChatMessage::Thinking(msg) => msg.render_into(area, display, max_lines, buf, theme),
            ChatMessage::Compaction(msg) => msg.render_into(area, buf, theme),
            ChatMessage::System(msg) => msg.render_into(area, buf, theme),
            ChatMessage::Hook(msg) => msg.render_into(area, hook_display, buf, theme),
            ChatMessage::SkillActivation(msg) => msg.render_into(area, buf, theme),
        }
    }
}

/// Height of the blank separator row between messages.
const SEPARATOR_HEIGHT: usize = 1;

/// Chat area renderer.
#[derive(Debug)]
pub struct ChatRenderer;

impl ChatRenderer {
    pub fn render(frame: &mut Frame, area: Rect, app: &mut App, theme: &Theme) {
        let messages = &app.chat.chat_messages;
        if messages.is_empty() {
            WelcomeScreen::render(frame, area, app, theme);
            app.chat.last_total_height = 0;
            app.chat.last_viewport_height = area.height as usize;
            return;
        }

        if area.height == 0 {
            return;
        }

        let display = app.config.thinking_display;
        let max_lines = app.config.config.ui.thinking_max_lines;
        let hook_display = app.config.hook_display;
        let rtk_active = app.config.config.rtk.enabled && app.config.rtk_state.is_available();
        let width = area.width as usize;

        // Compute heights for all messages
        let msg_heights: usize = messages
            .iter()
            .map(|m| m.height(width, display, max_lines, hook_display, theme))
            .sum();
        // Separators between messages (N messages → N-1 separators)
        let separator_height: usize = messages.len().saturating_sub(1) * SEPARATOR_HEIGHT;
        let total_height = msg_heights + separator_height;

        // Cache heights for scroll methods
        app.chat.last_total_height = total_height;
        app.chat.last_viewport_height = area.height as usize;

        if total_height == 0 {
            return;
        }

        // Auto-scroll: if flag is set, scroll to bottom using actual heights.
        // This avoids the stale-height bug where scroll_to_bottom() is called
        // before render updates last_total_height.
        let viewport_height = area.height as usize;
        if app.chat.auto_scroll {
            app.chat.scroll_offset = total_height.saturating_sub(viewport_height);
            app.chat.auto_scroll = false; // clear after processing
        }

        // Compute viewport
        let max_offset = total_height.saturating_sub(viewport_height);
        let scroll_offset = app.chat.scroll_offset.min(max_offset);
        let visible_start = scroll_offset;
        let visible_end = scroll_offset + viewport_height;

        // Walk messages, track cumulative Y, render visible ones
        let mut row: usize = 0;
        for (i, message) in messages.iter().enumerate() {
            // Render separator before every message except the first
            let sep_row = (i > 0).then(|| {
                row += SEPARATOR_HEIGHT;
                row - SEPARATOR_HEIGHT
            });

            let msg_h = message.height(width, display, max_lines, hook_display, theme);
            let msg_end = row + msg_h;

            // Skip if entirely outside viewport
            if msg_end <= visible_start
                && sep_row.map_or(0, |sr| sr + SEPARATOR_HEIGHT) <= visible_start
            {
                row = msg_end;
                continue;
            }

            // Render separator if visible
            if let Some(sr) = sep_row {
                let sep_visible_start = sr.max(visible_start);
                let sep_visible_end = (sr + SEPARATOR_HEIGHT).min(visible_end);
                if sep_visible_end > sep_visible_start {
                    let sep_y = (sep_visible_start - scroll_offset) as u16;
                    let sep_h = (sep_visible_end - sep_visible_start) as u16;
                    let sep_area = Rect {
                        x: area.x,
                        y: area.y + sep_y,
                        width: area.width,
                        height: sep_h,
                    };
                    let bg_fill =
                        Paragraph::new(Text::default()).style(Style::default().bg(theme.bg));
                    frame.render_widget(bg_fill, sep_area);
                }
            }

            // Render message (with clipping)
            let msg_visible_start = row.max(visible_start);
            let msg_visible_end = msg_end.min(visible_end);
            if msg_visible_end > msg_visible_start {
                let clip_rows = msg_visible_end - msg_visible_start;
                let skip_rows = msg_visible_start - row;

                // Render into temp buffer, then copy visible rows
                let temp_area = Rect {
                    x: 0,
                    y: 0,
                    width: area.width,
                    height: msg_h as u16,
                };
                let mut buf = Buffer::empty(temp_area);
                message.render_into(temp_area, display, max_lines, hook_display, &mut buf, theme, rtk_active);

                // Copy visible rows to frame
                let target_y = (msg_visible_start - scroll_offset) as u16;
                let fbuf = frame.buffer_mut();
                for r in 0..clip_rows {
                    let src_row = skip_rows + r;
                    if src_row < msg_h {
                        for c in 0..area.width as usize {
                            fbuf[(area.x + c as u16, target_y + r as u16)] =
                                buf[(c as u16, src_row as u16)].clone();
                        }
                    }
                }
            }

            row = msg_end;
        }

        // Process stretch effects for new messages
        Self::process_effects(frame, area, app, theme);
    }

    /// Build total height for scroll calculations.
    #[must_use]
    pub fn total_height(messages: &[ChatMessage], width: usize, display: ThinkingDisplay, max_lines: usize, hook_display: HookDisplay, theme: &Theme) -> usize {
        let msg_heights: usize = messages.iter().map(|m| m.height(width, display, max_lines, hook_display, theme)).sum();
        let separator_height = messages.len().saturating_sub(1) * SEPARATOR_HEIGHT;
        msg_heights + separator_height
    }

    /// Add stretch effects for Pending tool calls within their animation window.
    fn process_effects(_frame: &mut Frame, area: Rect, app: &mut App, theme: &Theme) {
        let messages = &mut app.chat.chat_messages;
        let display = app.config.thinking_display;
        let max_lines = app.config.config.ui.thinking_max_lines;
        let hook_display = app.config.hook_display;
        let width = area.width as usize;
        let scroll_offset = app.chat.scroll_offset;

        // Compute max offset for clamping
        let viewport_height = area.height as usize;
        let total_height = Self::total_height(messages, width, display, max_lines, hook_display, theme);
        let max_offset = total_height.saturating_sub(viewport_height);
        let clamped_offset = scroll_offset.min(max_offset);
        let visible_start = clamped_offset;
        let visible_end = clamped_offset + viewport_height;

        // Add stretch effects only for Pending tool calls within animation window.
        // Effect is added exactly once (effect_added flag) to avoid overlapping effects.
        let mut row: usize = 0;
        for (i, message) in messages.iter_mut().enumerate() {
            if i > 0 {
                row += SEPARATOR_HEIGHT;
            }
            let msg_h = message.height(width, display, max_lines, hook_display, theme);
            let msg_end = row + msg_h;

            // Add stretch effect once for Pending tool calls entering animation window
            if let ChatMessage::ToolCall(ToolCallMessage {
                variant: ToolCallVariant::Pending { animation_start, effect_added, .. },
            }) = message
                && !*effect_added
                && animation_start
                    .as_ref()
                    .is_some_and(|s| Instant::now().duration_since(*s) < ANIMATION_DURATION)
            {
                *effect_added = true;
                let vis_start = row.max(visible_start);
                let vis_end = msg_end.min(visible_end);
                if vis_end > vis_start {
                    let effect_y = area.y + (vis_start - clamped_offset) as u16;
                    let effect_h = (vis_end - vis_start) as u16;
                    let effect_area = Rect {
                        x: area.x,
                        y: effect_y,
                        width: area.width,
                        height: effect_h,
                    };
                    let fade = fx::fade_from(
                        theme.bg_dark,
                        theme.bg,
                        EffectTimer::from_ms(150, Interpolation::QuadOut),
                    )
                    .with_area(effect_area);
                    app.ui.effect_manager.add_effect(fade);
                }
            }

            row = msg_end;
        }
    }
}
