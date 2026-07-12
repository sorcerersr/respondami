//! Skill activation message rendering.
//!
//! Uses the `FilledHeaderBar` widget with bright purple color (139, 92, 246).

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Color;
use ratatui::text::Line;

use super::super::theme::Theme;
use ratatui_widgets::FilledHeaderBar;

/// Skill activation message displayed in the chat.
#[derive(Debug, Clone)]
pub struct SkillActivationMessage {
    pub skill_name: String,
}

impl SkillActivationMessage {
    /// Compute height: header bar + content line + bottom border.
    #[must_use]
    pub fn height(&self, _width: usize, _theme: &Theme) -> usize {
        3 // header + content + bottom border
    }

    /// Render the skill activation message using the `FilledHeaderBar` widget.
    ///
    /// Layout:
    /// ┌──────────────────────────────────────────┐
    /// │ Skill Activation                         │  ← filled header bar, white text on bright purple
    /// ├──────────────────────────────────────────┤
    /// │ refine                                   │  ← skill name in content area
    /// └──────────────────────────────────────────┘
    pub fn render_into(&self, area: Rect, buf: &mut Buffer, theme: &Theme) {
        if area.width == 0 || area.height == 0 {
            return;
        }

        let purple = Color::Rgb(139, 92, 246);

        let content_line = Line::from(ratatui::text::Span::styled(
            self.skill_name.clone(),
            theme.text_style(),
        ));

        let widget = FilledHeaderBar::new("Skill Activation")
            .header_color(purple)
            .border_color(purple)
            .content(vec![content_line])
            .collapsed(false)
            .content_bg(theme.bg)
            .content_fg(Color::White);

        widget.render_into(area, buf);
    }
}
