//! Welcome screen rendering.
//!
//! Shown when the chat has no messages. Contains ASCII art, gradient text,
//! and info panels (skills, hooks, context).
//!
//! Rust guideline compliant 2026-07-08

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::style::Color;
use ratatui::text::{Line, Span};
use ratatui::Frame;
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use crate::tui::app::App;
use crate::tui::theme::Theme;
use ratatui_widgets::FilledHeaderBar;

/// Welcome screen renderer.
///
/// Displays ASCII art, subtitle, and info panels when the chat is empty.
pub struct WelcomeScreen;

impl WelcomeScreen {
    /// ASCII art for the welcome screen. 4 rows, 77 display columns wide.
    const WELCOME_ART: [&str; 4] = [
        "██████▄ ▄██████ ▄█████▄ ██████▄ ▄█████▄ ▄█████▄ ██████▄ ▄█████▄ ▄██████▄ ▐██▌",
        "██   ██ ██▄▄▄▄  ██▄▄▄▄  ██▄▄▄██ ██   ██ ██   ██ ██   ██ ██▄▄▄██ ██ ██ ██  ██ ",
        "██████  ██▀▀▀▀   ▀▀▀▀██ ██▀▀▀▀  ██   ██ ██   ██ ██   ██ ██▀▀▀██ ██ ██ ██  ██ ",
        "██  ▀██ ▀██████ ▀█████▀ ██      ▀█████▀ ██   ██ ██████▀ ██   ██ ██ ██ ██ ▐██▌",
    ];

    /// Gradient start color (white/text: #d1d7e0).
    const GRADIENT_START: (u8, u8, u8) = (0xd1, 0xd7, 0xe0);
    /// Gradient end color (accent blue: #478be6).
    const GRADIENT_END: (u8, u8, u8) = (0x47, 0x8b, 0xe6);

    /// Minimum width for two-column info panel layout.
    const TWO_COLUMN_MIN_WIDTH: u16 = 64;
    /// Minimum width for three-column info panel layout.
    const THREE_COLUMN_MIN_WIDTH: u16 = 110;

    /// Linearly interpolate between two RGB colors.
    fn interpolate_color(start: (u8, u8, u8), end: (u8, u8, u8), t: f64) -> Color {
        let t = t.clamp(0.0, 1.0);
        let r = (f64::from(start.0) + (f64::from(end.0) - f64::from(start.0)) * t) as u8;
        let g = (f64::from(start.1) + (f64::from(end.1) - f64::from(start.1)) * t) as u8;
        let b = (f64::from(start.2) + (f64::from(end.2) - f64::from(start.2)) * t) as u8;
        Color::Rgb(r, g, b)
    }

    /// Render a centered line of text at the given row within the area.
    fn render_centered_line(buf: &mut Buffer, area: Rect, row: u16, text: &str, style: Style) {
        let text_width = text.width();
        let start_col = (area.width as usize).saturating_sub(text_width).saturating_div(2) as u16;
        let mut col = start_col;
        for ch in text.chars() {
            let ch_width = ch.width().unwrap_or(1);
            if col < area.width {
                buf[(col, row)].set_symbol(ch.to_string().as_str()).set_style(style);
            }
            col += ch_width as u16;
        }
    }

    /// Render a right-aligned line of text at the given row within the area.
    fn render_right_aligned_line(buf: &mut Buffer, area: Rect, row: u16, text: &str, style: Style) {
        let text_width = text.width();
        let start_col = (area.width as usize).saturating_sub(text_width) as u16;
        let mut col = start_col;
        for ch in text.chars() {
            let ch_width = ch.width().unwrap_or(1);
            if col < area.width {
                buf[(col, row)].set_symbol(ch.to_string().as_str()).set_style(style);
            }
            col += ch_width as u16;
        }
    }

    /// Render gradient-colored big text into the buffer, clipped to area width.
    /// Returns the number of display columns actually used (for centering calculation).
    fn render_big_text(buf: &mut Buffer, area: Rect, start_y: u16) -> usize {
        let max_width = area.width as usize;
        let mut total_display_width = 0usize;

        // Measure max display width across all rows
        for line in &Self::WELCOME_ART {
            let w: usize = line.chars().map(|c| c.width().unwrap_or(1)).sum();
            total_display_width = total_display_width.max(w);
        }

        // Compute horizontal offset for centering (0 if clipped)
        let x_offset = if total_display_width <= max_width {
            (max_width - total_display_width) / 2
        } else {
            0
        };

        for (row_idx, line) in Self::WELCOME_ART.iter().enumerate() {
            let y = start_y + row_idx as u16;
            if y >= area.y + area.height {
                break;
            }

            let mut col_pos = 0usize;
            for ch in line.chars() {
                let ch_width = ch.width().unwrap_or(1);
                let draw_col = x_offset + col_pos;

                // Clip: stop if character starts beyond area width
                if draw_col >= max_width {
                    break;
                }

                // Compute gradient color based on position within the full art width
                let t = if total_display_width > 0 {
                    col_pos as f64 / total_display_width as f64
                } else {
                    0.0
                };
                let color = Self::interpolate_color(Self::GRADIENT_START, Self::GRADIENT_END, t);

                // Draw character (may span multiple columns for wide chars)
                for sub_col in 0..ch_width {
                    let cell_col = draw_col + sub_col;
                    if cell_col < max_width {
                        let cell = &mut buf[(cell_col as u16, y)];
                        if sub_col == 0 {
                            cell.set_symbol(ch.to_string().as_str());
                        }
                        cell.set_fg(color);
                        cell.set_bg(Color::Rgb(0x21, 0x28, 0x30));
                    }
                }

                col_pos += ch_width;
            }
        }

        total_display_width
    }

    /// Build content lines for the skills panel.
    #[must_use]
    pub(crate) fn build_skills_content(
        skills: &[crate::skills::Skill],
        dim_color: Color,
    ) -> Vec<Line<'static>> {
        if skills.is_empty() {
            vec![Line::from(Span::styled(
                "No skills loaded",
                Style::default().fg(dim_color),
            ))]
        } else {
            skills.iter().map(|s| Line::from(s.name.clone())).collect()
        }
    }

    /// Build content lines for the context panel.
    #[must_use]
    pub(crate) fn build_context_content(
        cwd: &std::path::Path,
        agents_md_path: &Option<std::path::PathBuf>,
        dim_color: Color,
    ) -> Vec<Line<'static>> {
        match agents_md_path {
            Some(path) => {
                let display = path.strip_prefix(cwd).unwrap_or(path.as_path()).display().to_string();
                vec![Line::from(display)]
            }
            None => vec![Line::from(Span::styled(
                "No AGENTS.md found",
                Style::default().fg(dim_color),
            ))],
        }
    }

    /// Build content lines for the hooks panel.
    #[must_use]
    pub(crate) fn build_hooks_content(
        registry: &crate::hooks::HookRegistry,
        dim_color: Color,
    ) -> Vec<Line<'static>> {
        use crate::hooks::HookEvent;
        let mut lines = Vec::new();
        for event in HookEvent::all() {
            let hooks = registry.hooks(*event);
            if hooks.is_empty() {
                continue;
            }
            lines.push(Line::from(format!("{event}:")));
            for hook in hooks {
                lines.push(Line::from(format!("  {}", hook.name)));
            }
        }
        if lines.is_empty() {
            lines.push(Line::from(Span::styled(
                "No hooks configured",
                Style::default().fg(dim_color),
            )));
        }
        lines
    }

    /// Render the welcome screen into the given area.
    pub fn render(frame: &mut Frame, area: Rect, app: &App, theme: &Theme) {
        let skills = &app.config.skills;
        let agents_md_path = &app.config.agents_md_path;
        let cwd = &app.config.cwd;
        let hook_registry = &app.config.hook_registry;

        // Build panel content
        let skills_content = Self::build_skills_content(skills, theme.text_dim);
        let hooks_content = Self::build_hooks_content(hook_registry, theme.text_dim);
        let context_content = Self::build_context_content(cwd, agents_md_path, theme.text_dim);

        // Count content lines for height calculation
        let skills_line_count = if skills.is_empty() { 1 } else { skills.len() };
        let hooks_line_count = hooks_content.len();
        let context_line_count = 1;
        let panel_content_height = skills_line_count.max(hooks_line_count).max(context_line_count);
        let panel_total_height = 2 + panel_content_height; // header(2) + content

        // Compute total content height for vertical centering
        let big_text_height = 4;
        let subtitle_height = 1;
        let help_height = 2; // Two lines

        let three_column = area.width >= Self::THREE_COLUMN_MIN_WIDTH;
        let two_column = area.width >= Self::TWO_COLUMN_MIN_WIDTH;

        let (panels_height, gaps) = if three_column {
            (panel_total_height, 4) // blank rows: after big_text, after subtitle, after panels, before help(×2)
        } else if two_column {
            (panel_total_height, 5) // blank rows: after big_text, after subtitle, after panels(×2), before help
        } else {
            // Stacked: 3 panels × height + 2 gaps between
            (panel_total_height * 3 + 2, 4) // blank rows: after big_text, after subtitle, after panels, before help(×2)
        };

        let total_content_height = big_text_height + subtitle_height + panels_height + help_height + gaps;
        let top_pad = if area.height as usize > total_content_height {
            (area.height as usize - total_content_height) / 2
        } else {
            0
        };

        let buf = frame.buffer_mut();
        let width = area.width as usize;

        // Fill background
        for y in 0..area.height {
            for x in 0..area.width {
                buf[(x, area.y + y)].set_bg(theme.bg);
            }
        }

        // Version number in top-right corner
        let version = format!("v{}", env!("CARGO_PKG_VERSION"));
        Self::render_right_aligned_line(buf, area, area.y, &version, theme.text_muted_style());

        let mut row = area.y + top_pad as u16;

        // 1. Big text with gradient
        Self::render_big_text(buf, area, row);
        row += big_text_height as u16;

        // 2. Blank row
        row += 1;

        // 3. Subtitle (centered)
        let subtitle = "A privacy-first coding agent. One binary, no telemetry, skills-driven.";
        Self::render_centered_line(buf, area, row, subtitle, theme.text_muted_style());
        row += 1;

        // 4. Blank row
        row += 1;

        // 5. Info panels
        if three_column {
            // Three-column layout: equal width, 1-col gaps, centered as group
            let context_display = match agents_md_path {
                Some(p) => p.strip_prefix(cwd).unwrap_or(p.as_path()).display().to_string(),
                None => "No AGENTS.md found".to_string(),
            };
            let skills_max_width = if skills.is_empty() {
                "No skills loaded".width()
            } else {
                skills.iter().map(|s| s.name.width()).max().unwrap_or(20)
            };
            let hooks_max_width = hooks_content.iter().map(ratatui_md::Line::width).max().unwrap_or(20);
            let min_panel_width = skills_max_width.max(hooks_max_width).max(context_display.width()) + 2;
            let panel_width = min_panel_width.min((width - 4) / 3);
            let total_panels_width = panel_width * 3 + 2; // 2 gaps
            let panels_start_x = (width - total_panels_width).saturating_div(2);

            let left_area = Rect {
                x: area.x + panels_start_x as u16,
                y: row,
                width: panel_width as u16,
                height: panel_total_height as u16,
            };
            let mid_area = Rect {
                x: area.x + (panels_start_x + panel_width + 1) as u16,
                y: row,
                width: panel_width as u16,
                height: panel_total_height as u16,
            };
            let right_area = Rect {
                x: area.x + (panels_start_x + panel_width * 2 + 2) as u16,
                y: row,
                width: panel_width as u16,
                height: panel_total_height as u16,
            };

            // Skills panel (purple header)
            let skills_widget = FilledHeaderBar::new("Skills")
                .header_color(Color::Rgb(0xb0, 0x83, 0xf0))
                .border_color(Color::Rgb(0xb0, 0x83, 0xf0))
                .content(skills_content)
                .content_bg(theme.bg)
                .content_fg(theme.text_muted);
            skills_widget.render_into(left_area, buf);

            // Hooks panel (green header)
            let hooks_widget = FilledHeaderBar::new("Hooks")
                .header_color(Color::Rgb(0x3f, 0xb9, 0x50))
                .border_color(Color::Rgb(0x3f, 0xb9, 0x50))
                .content(hooks_content)
                .content_bg(theme.bg)
                .content_fg(theme.text_muted);
            hooks_widget.render_into(mid_area, buf);

            // Context panel (cyan header)
            let context_widget = FilledHeaderBar::new("Context")
                .header_color(theme.cyan)
                .border_color(theme.cyan)
                .content(context_content)
                .content_bg(theme.bg)
                .content_fg(theme.text_muted);
            context_widget.render_into(right_area, buf);

            row += panel_total_height as u16;
        } else if two_column {
            // Two-column layout: Skills | Context (no hooks panel)
            let context_display = match agents_md_path {
                Some(p) => p.strip_prefix(cwd).unwrap_or(p.as_path()).display().to_string(),
                None => "No AGENTS.md found".to_string(),
            };
            let skills_max_width = if skills.is_empty() {
                "No skills loaded".width()
            } else {
                skills.iter().map(|s| s.name.width()).max().unwrap_or(20)
            };
            let min_panel_width = skills_max_width.max(context_display.width()) + 2;
            let panel_width = min_panel_width.min((width - 2) / 2);
            let total_panels_width = panel_width * 2 + 1;
            let panels_start_x = (width - total_panels_width).saturating_div(2);

            let left_area = Rect {
                x: area.x + panels_start_x as u16,
                y: row,
                width: panel_width as u16,
                height: panel_total_height as u16,
            };
            let right_area = Rect {
                x: area.x + (panels_start_x + panel_width + 1) as u16,
                y: row,
                width: panel_width as u16,
                height: panel_total_height as u16,
            };

            let skills_widget = FilledHeaderBar::new("Skills")
                .header_color(Color::Rgb(0xb0, 0x83, 0xf0))
                .border_color(Color::Rgb(0xb0, 0x83, 0xf0))
                .content(skills_content)
                .content_bg(theme.bg)
                .content_fg(theme.text_muted);
            skills_widget.render_into(left_area, buf);

            let context_widget = FilledHeaderBar::new("Context")
                .header_color(theme.cyan)
                .border_color(theme.cyan)
                .content(context_content)
                .content_bg(theme.bg)
                .content_fg(theme.text_muted);
            context_widget.render_into(right_area, buf);

            row += panel_total_height as u16;
        } else {
            // Single-column layout: stacked, full width
            let skills_widget = FilledHeaderBar::new("Skills")
                .header_color(Color::Rgb(0xb0, 0x83, 0xf0))
                .border_color(Color::Rgb(0xb0, 0x83, 0xf0))
                .content(skills_content)
                .content_bg(theme.bg)
                .content_fg(theme.text_muted);
            let skills_h = (2 + skills_line_count) as u16;
            let skills_area = Rect {
                x: area.x,
                y: row,
                width: area.width,
                height: skills_h,
            };
            skills_widget.render_into(skills_area, buf);
            row += skills_h;

            row += 1;

            let hooks_widget = FilledHeaderBar::new("Hooks")
                .header_color(Color::Rgb(0x3f, 0xb9, 0x50))
                .border_color(Color::Rgb(0x3f, 0xb9, 0x50))
                .content(hooks_content)
                .content_bg(theme.bg)
                .content_fg(theme.text_muted);
            let hooks_h = (2 + hooks_line_count) as u16;
            let hooks_area = Rect {
                x: area.x,
                y: row,
                width: area.width,
                height: hooks_h,
            };
            hooks_widget.render_into(hooks_area, buf);
            row += hooks_h;

            row += 1;

            let context_widget = FilledHeaderBar::new("Context")
                .header_color(theme.cyan)
                .border_color(theme.cyan)
                .content(context_content)
                .content_bg(theme.bg)
                .content_fg(theme.text_muted);
            let context_h = (2 + context_line_count) as u16;
            let context_area = Rect {
                x: area.x,
                y: row,
                width: area.width,
                height: context_h,
            };
            context_widget.render_into(context_area, buf);
            row += context_h;
        }

        // 6. Blank row
        row += 1;

        // 7. Help text (two lines, centered, brighter)
        let help_style = theme.text_muted_style();
        Self::render_centered_line(buf, area, row, "Type a message, @ for file references, or / to trigger skills.", help_style);
        row += 1;
        Self::render_centered_line(buf, area, row, "Ctrl+D to exit  •  Ctrl+G for command palette", help_style);
    }
}
