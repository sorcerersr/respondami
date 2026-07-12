//! Main layout and rendering for the TUI frame.
//!
//! Splits the terminal into chat area, input area, and status bar.
//! Renders popups (init, session select, command palette, help, autocomplete) on top.
//!
//! Rust guideline compliant 2026-02-21

use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::Clear;
use ratatui::Frame;
use tachyonfx::{fx, EffectTimer, Interpolation};

use super::app::App;
use super::editor::FileMatch;
use super::mode::PopupType;
use super::theme::Theme;
use ratatui_widgets::{
    AutocompletePopup, CommandPaletteOverlay, PanelOverlay,
};

/// Main layout renderer.
#[derive(Debug, Default)]
pub struct LayoutRenderer;

impl LayoutRenderer {
    /// Maximum text rows visible in the input area.
    const MAX_TEXT_ROWS: usize = 6;

    /// Calculate the height of the input area.
    /// Just the visible text rows (1 to `MAX_TEXT_ROWS`). Spacing comes from chat area above.
    #[must_use]
    pub fn input_area_height(app: &App) -> usize {
        Self::input_area_height_for(&app.editor.input_buffer, app.ui.terminal_width)
    }

    /// Pure calculation: input area height from buffer text and wrap width.
    /// Used by `input_area_height()` and testable in isolation.
    pub(crate) fn input_area_height_for(input: &str, wrap_width: usize) -> usize {
        use super::editor::build_visual_lines;
        let width = wrap_width.max(1);
        let visual_lines = build_visual_lines(input, width);
        let total_visual = visual_lines.len().max(1);
        total_visual.clamp(1, Self::MAX_TEXT_ROWS)
    }

    /// Add fade-in animation for overlay popups.
    fn maybe_add_popup_fade_effect(app: &mut App, popup_type: PopupType, popup_area: Rect, theme: &Theme) {
        if let Some((current_type, _)) = app.modal.popup_animation
            && current_type == popup_type
        {
            return;
        }
        app.modal.popup_animation = Some((popup_type, std::time::Instant::now()));
        let fade = fx::fade_from(
            theme.bg_dark,
            theme.bg,
            EffectTimer::from_ms(250, Interpolation::QuadOut),
        )
        .with_area(popup_area);
        app.ui.effect_manager.add_effect(fade);
    }

    /// Clear popup animation state (called when a popup closes).
    fn clear_popup_animation(app: &mut App) {
        app.modal.popup_animation = None;
    }

    /// Render the full application layout.
    ///
    /// Layout structure:
    /// ┌─────────────────────────────────────┐
    /// │ Chat Area (Min(1))                   │
    /// │ ...                                  │
    /// │ > Prompt Input (+ autocomplete)      │
    /// │ ──────────────────────────────────── │
    /// │ ◐ Streaming ◐ │ ⌂ cwd │ model │ ... │
    /// └─────────────────────────────────────┘
    pub fn render(frame: &mut Frame, app: &mut App, theme: &Theme) {
        let area = frame.area();

        // Track render timing — computed once per frame at the layout level.
        let elapsed_ms = app.ui.last_render_time.elapsed().as_millis() as u32;
        app.ui.last_render_time = std::time::Instant::now();
        app.ui.terminal_width = area.width as usize;

        // Set input area width for visual-line cursor movement.
        // Inner width = total width minus 2 (left/right borders).
        app.ui.input_area_width = (area.width as usize).saturating_sub(2);

        // Calculate input area height: 1 row for prompt + autocomplete dropdown when active
        let input_height = Self::input_area_height(app);

        // Build constraints: always 3-row layout (chat | input | status)
        let vertical = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Min(1),  // Chat area (fills remaining)
                Constraint::Length(input_height as u16),  // Prompt input (+ autocomplete)
                Constraint::Length(1),  // Status bar
            ])
            .split(area);
        let (chat_row, input_row, status_row) = (vertical[0], vertical[1], vertical[2]);

        // Chat area
        Self::render_chat_area(frame, chat_row, app, theme);


        // Prompt input area
        Self::render_input_area(frame, input_row, app, theme);

        // Status bar
        super::status_bar::render_status_bar(frame, status_row, app, theme);

        // Init popup overlay (drawn on top of chat area)
        if app.modal.state == super::mode::AppState::InitPopup {
            Self::render_init_popup(frame, chat_row, app, theme);
        }

        // Session selector overlay (drawn on top of chat area)
        if app.modal.state == super::mode::AppState::SessionSelect {
            Self::render_session_select_popup(frame, chat_row, app, theme);
        }

        // Command palette overlay (drawn on top of chat/input boundary)
        if matches!(app.modal.state, super::mode::AppState::CommandPalette) {
            Self::render_command_palette(frame, chat_row, input_row, app, theme);
        }

        // Help popup overlay (drawn on top of chat area)
        if app.modal.state == super::mode::AppState::HelpPopup {
            Self::render_help_popup(frame, chat_row, app, theme);
        }

        // File autocomplete popup overlay (drawn on top of chat/input boundary)
        if let Some((file_matches, file_selected, file_scroll, file_show_hidden)) = {
            match &app.editor.autocomplete_mode {
                super::mode::AutocompleteMode::File {
                    matches,
                    selected,
                    scroll_offset,
                    show_hidden,
                } if !matches.is_empty() => {
                    Some((matches.clone(), *selected, *scroll_offset, *show_hidden))
                }
                _ => None,
            }
        } {
            // Debug: log popup rendering params
            #[cfg(debug_assertions)]
            tracing::debug!(
                "[popup] matches={} selected={} scroll={} show_hidden={} input_buf={:?}",
                file_matches.len(),
                file_selected,
                file_scroll,
                file_show_hidden,
                app.editor.input_buffer
            );
            Self::render_file_autocomplete_popup(
                frame,
                chat_row,
                input_row,
                app,
                &file_matches,
                file_selected,
                file_scroll,
                file_show_hidden,
                theme,
            );
        }

        // Skill autocomplete popup overlay (drawn on top of chat/input boundary)
        if let Some((skill_matches, skill_selected, skill_scroll)) = {
            match &app.editor.autocomplete_mode {
                super::mode::AutocompleteMode::Skill {
                    matches,
                    selected,
                    scroll_offset,
                } if !matches.is_empty() => {
                    Some((matches.clone(), *selected, *scroll_offset))
                }
                _ => None,
            }
        } {
            Self::render_skill_autocomplete_popup(
                frame,
                chat_row,
                input_row,
                app,
                &skill_matches,
                skill_selected,
                skill_scroll,
                theme,
            );
        }

        // Clear popup animation state when no popup is active
        let any_autocomplete_popup = matches!(&app.editor.autocomplete_mode,
            super::mode::AutocompleteMode::Skill { matches, .. } if !matches.is_empty())
            || matches!(&app.editor.autocomplete_mode, super::mode::AutocompleteMode::File { matches, .. } if !matches.is_empty());
        let any_popup_active = app.modal.state == super::mode::AppState::InitPopup
            || app.modal.state == super::mode::AppState::SessionSelect
            || app.modal.state == super::mode::AppState::CommandPalette
            || app.modal.state == super::mode::AppState::HelpPopup
            || any_autocomplete_popup;
        if app.modal.popup_animation.is_some() && !any_popup_active {
            Self::clear_popup_animation(app);
        }

        // Process all effects (tool call stretch + popup expand) over full frame area
        if app.ui.effect_manager.is_running() && elapsed_ms > 0 {
            let duration = tachyonfx::Duration { milliseconds: elapsed_ms };
            app.ui.effect_manager
                .process_effects(duration, frame.buffer_mut(), area);
        }
    }

    /// Render the AGENTS.md generation popup.
    ///
    /// Centered Block with Yes/No options, drawn on top of the chat area.
    fn render_init_popup(frame: &mut Frame, area: Rect, app: &mut App, theme: &Theme) {
        let popup_height = 8;
        let popup_width = 52;
        let popup_height = popup_height.min(area.height);
        let popup_width = popup_width.min(area.width);

        let popup_area = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([
                Constraint::Min(0),
                Constraint::Length(popup_width),
                Constraint::Min(0),
            ])
            .split(area)[1];

        let popup_area = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Min(0),
                Constraint::Length(popup_height),
                Constraint::Min(0),
            ])
            .split(popup_area)[1];

        // Ensure minimum viable size
        if popup_area.width < 20 || popup_area.height < 4 {
            return;
        }

        // Add sweep animation on first render
        Self::maybe_add_popup_fade_effect(app, PopupType::Init, popup_area, theme);

        let yes_label = if app.modal.popup_selection == 0 {
            " ▸ Yes, generate AGENTS.md "
        } else {
            "   Yes, generate AGENTS.md "
        };
        let no_label = if app.modal.popup_selection == 1 {
            " ▸ No, not now "
        } else {
            "   No, not now "
        };

        let yes_style = if app.modal.popup_selection == 0 {
            theme.panel_selected_style()
        } else {
            theme.panel_content_style()
        };
        let no_style = if app.modal.popup_selection == 1 {
            theme.panel_selected_style()
        } else {
            theme.panel_content_style()
        };

        let lines = vec![
            Line::from(Span::styled(
                "Would you like to generate an AGENTS.md",
                theme.panel_content_style(),
            )),
            Line::from(Span::styled(
                "for this project?",
                theme.panel_content_style(),
            )),
            Line::from(""),
            Line::from(Span::styled(yes_label, yes_style)),
            Line::from(Span::styled(no_label, no_style)),
            Line::from(""),
            Line::from(Span::styled(
                "Enter: confirm  Esc: dismiss",
                Style::default().fg(theme.text_dim).bg(theme.bg_light),
            )),
        ];

        let overlay = PanelOverlay::new(" Generate AGENTS.md? ")
            .border_style(theme.panel_border_style())
            .title_style(theme.panel_title_style())
            .content(lines)
            .content_bg(theme.bg_light)
            .content_style(theme.panel_content_style())
            .selected_style(theme.panel_selected_style());

        frame.render_widget(Clear, popup_area);
        frame.render_widget(overlay, popup_area);
    }

    /// Render the session selector popup as a large centered overlay.
    ///
    /// Fills the chat area with 4-char padding on all sides.
    fn render_session_select_popup(frame: &mut Frame, area: Rect, app: &mut App, theme: &Theme) {
        use chrono::DateTime;

        if app.modal.session_select_matches.is_empty() {
            return;
        }

        // 4-char padding on all sides
        let padding = 4u16;
        if area.width < padding * 2 + 10 || area.height < padding * 2 + 4 {
            return;
        }

        let popup_area = Rect {
            x: area.x + padding,
            y: area.y + padding,
            width: area.width - padding * 2,
            height: area.height - padding * 2,
        };

        // Add sweep animation on first render (before borrowing sessions)
        Self::maybe_add_popup_fade_effect(app, PopupType::SessionSelect, popup_area, theme);

        let sessions = &app.modal.session_select_matches;

        let format_timestamp = |ts: &str| -> String {
            DateTime::parse_from_rfc3339(ts).map_or_else(|_| "unknown".into(), |dt| dt.format("%Y-%m-%d %H:%M").to_string())
        };

        let text: Vec<ratatui::text::Line> = sessions.iter().enumerate().map(|(i, s)| {
            let prefix = if i == app.modal.session_select_index { "↑ " } else { "  " };
            let date = format_timestamp(&s.timestamp);
            let msg_preview = s.first_message.chars().take(40).collect::<String>();
            let label = format!(
                "{}{} · \"{}\" · {} messages",
                prefix, date, msg_preview, s.message_count
            );
            ratatui::text::Line::from(ratatui::text::Span::styled(label, theme.panel_content_style()))
        }).collect();

        let overlay = PanelOverlay::new(" Resume Session ")
            .border_style(theme.panel_border_style())
            .title_style(theme.panel_title_style())
            .content(text)
            .content_bg(theme.bg_light)
            .selected(app.modal.session_select_index)
            .content_style(theme.panel_content_style())
            .selected_style(theme.panel_selected_style());

        frame.render_widget(Clear, popup_area);
        frame.render_widget(overlay, popup_area);
    }

    /// Render the file autocomplete popup as a centered overlay.
    ///
    /// Horizontally centered, anchored above the input row, grows upward.
    /// Uses front-truncation for long paths and a footer row to toggle hidden files.
    #[expect(clippy::too_many_arguments, reason = "render signature unified across all message types")]
    fn render_file_autocomplete_popup(
        frame: &mut Frame,
        chat_area: Rect,
        input_row: Rect,
        app: &mut App,
        matches: &[FileMatch],
        selected: usize,
        scroll_offset: usize,
        show_hidden: bool,
        theme: &Theme,
    ) {
        use super::editor::FILE_POPUP_PAGE_SIZE;

        // Popup dimensions
        let visible_count = matches.len().min(FILE_POPUP_PAGE_SIZE);
        // +2 for borders, +2 for footer (divider + hint line)
        let popup_height = (visible_count + 4) as u16;

        // Width: 75% of chat area, clamped between 40 and 120
        let popup_width = (f64::from(chat_area.width) * 0.75) as u16;
        let popup_width = popup_width.clamp(40, 120).min(chat_area.width);

        // Clamp height to available space
        let space_above = input_row.y.saturating_sub(chat_area.y);
        let popup_height = popup_height.min(space_above);
        if popup_height < 4 {
            // Not enough space: need at least 4 (top border + 1 item + divider + footer + bottom border)
            return;
        }

        // Horizontal centering
        let popup_x = chat_area.x + (chat_area.width - popup_width) / 2;
        let popup_y = input_row.y - popup_height;
        let popup_y = popup_y.max(chat_area.y);

        let popup_area = Rect {
            x: popup_x,
            y: popup_y,
            width: popup_width,
            height: popup_height,
        };

        // Add sweep animation on first render
        Self::maybe_add_popup_fade_effect(app, PopupType::FileAutocomplete, popup_area, theme);

        // Build display items with front-truncation
        let inner_width = (popup_width - 2).max(1) as usize;
        let items: Vec<String> = matches.iter().map(|m| m.display_truncated(inner_width)).collect();

        // Footer hint line
        let footer_text = if show_hidden {
            "hide hidden files Ctrl+."
        } else {
            "show hidden files Ctrl+."
        };
        let footer = Line::from(Span::styled(
            footer_text.to_string(),
            Style::default().fg(theme.text_dim),
        ));

        let popup = AutocompletePopup::new()
            .items(items)
            .selected(selected)
            .scroll_offset(scroll_offset)
            .max_height(FILE_POPUP_PAGE_SIZE)
            .centered(true)
            .truncate_front(true)
            .truncate_max_width(inner_width)
            .footer(footer)
            .border_style(theme.panel_border_style())
            .title_style(theme.panel_title_style())
            .content_bg(theme.bg_light)
            .text_color(theme.text)
            .accent_color(theme.accent)
            .title(" Files ");

        // Clear the popup area first, then render the list on top
        frame.render_widget(Clear, popup_area);
        frame.render_widget(popup, popup_area);
    }

    /// Render the skill autocomplete popup as a centered overlay.
    ///
    /// Horizontally centered, anchored above the input row, grows upward.
    #[expect(clippy::too_many_arguments, reason = "render signature unified across all message types")]
    fn render_skill_autocomplete_popup(
        frame: &mut Frame,
        chat_area: Rect,
        input_row: Rect,
        app: &mut App,
        matches: &[String],
        selected: usize,
        scroll_offset: usize,
        theme: &Theme,
    ) {
        use super::editor::FILE_POPUP_PAGE_SIZE;

        // Popup dimensions
        let visible_count = matches.len().min(FILE_POPUP_PAGE_SIZE);
        let popup_height = (visible_count + 2) as u16; // +2 for borders

        // Width: 75% of chat area, clamped between 40 and 120
        let popup_width = (f64::from(chat_area.width) * 0.75) as u16;
        let popup_width = popup_width.clamp(40, 120).min(chat_area.width);

        // Clamp height to available space
        let space_above = input_row.y.saturating_sub(chat_area.y);
        let popup_height = popup_height.min(space_above);
        if popup_height < 2 {
            return;
        }

        // Horizontal centering
        let popup_x = chat_area.x + (chat_area.width - popup_width) / 2;
        let popup_y = input_row.y - popup_height;
        let popup_y = popup_y.max(chat_area.y);

        let popup_area = Rect {
            x: popup_x,
            y: popup_y,
            width: popup_width,
            height: popup_height,
        };

        // Add sweep animation on first render
        Self::maybe_add_popup_fade_effect(app, PopupType::SkillAutocomplete, popup_area, theme);

        let popup = AutocompletePopup::new()
            .items(matches.to_vec())
            .selected(selected)
            .scroll_offset(scroll_offset)
            .max_height(FILE_POPUP_PAGE_SIZE)
            .centered(true)
            .border_style(theme.panel_border_style())
            .title_style(theme.panel_title_style())
            .content_bg(theme.bg_light)
            .text_color(theme.text)
            .accent_color(theme.accent)
            .title(" Skills ");

        // Clear the popup area first, then render the list on top
        frame.render_widget(Clear, popup_area);
        frame.render_widget(popup, popup_area);
    }


    /// Render the help popup as a centered overlay.
    ///
    /// Height is computed from content (~25 rows for current keybindings). Width is fixed at 60.
    fn render_help_popup(frame: &mut Frame, area: Rect, app: &mut App, theme: &Theme) {
        let accent = theme.accent;
        let content = theme.panel_content_style();

        let headline_span = |t: &'static str| Span::styled(t, Style::default().fg(theme.purple));
        let key_span = |t: &'static str| Span::styled(t, Style::default().fg(accent));
        let desc_span = |t: &'static str| Span::styled(t, content);

        let lines = vec![
            Line::from(""),
            Line::from(headline_span("  Navigation")),
            Line::from(vec![key_span("    PgUp / PgDown    "), desc_span("Scroll chat page")]),
            Line::from(vec![key_span("    j / k            "), desc_span("Navigate lists")]),
            Line::from(""),
            Line::from(headline_span("  Input")),
            Line::from(vec![key_span("    Enter            "), desc_span("Send message")]),
            Line::from(vec![key_span("    Shift+Enter      "), desc_span("New line")]),
            Line::from(vec![key_span("    Ctrl+G           "), desc_span("Command palette")]),
            Line::from(vec![key_span("    Ctrl+C / Ctrl+K  "), desc_span("Clear input")]),
            Line::from(vec![key_span("    Esc              "), desc_span("Clear input")]),
            Line::from(vec![key_span("    @                "), desc_span("File references")]),
            Line::from(vec![key_span("    /                "), desc_span("Skill references")]),
            Line::from(""),
            Line::from(headline_span("  Display")),
            Line::from(vec![key_span("    Ctrl+O / Ctrl+/  "), desc_span("Toggle reasoning")]),
            Line::from(vec![key_span("    Ctrl+T           "), desc_span("Toggle tool output")]),
            Line::from(""),
            Line::from(headline_span("  Shortcuts")),
            Line::from(vec![key_span("    F1               "), desc_span("This help")]),
            Line::from(vec![key_span("    Ctrl+D           "), desc_span("Quit")]),
            Line::from(""),
            Line::from(Span::styled("  Esc to close", Style::default().fg(theme.text_dim).bg(theme.bg_light))),
        ];

        // Dynamic height: content rows + 2 for borders
        let popup_height = (lines.len() + 2) as u16;
        let popup_width = 60;
        let popup_height = popup_height.min(area.height);
        let popup_width = popup_width.min(area.width);

        let popup_area = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([
                Constraint::Min(0),
                Constraint::Length(popup_width),
                Constraint::Min(0),
            ])
            .split(area)[1];

        let popup_area = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Min(0),
                Constraint::Length(popup_height),
                Constraint::Min(0),
            ])
            .split(popup_area)[1];

        // Ensure minimum viable size
        if popup_area.width < 30 || popup_area.height < 6 {
            return;
        }

        // Add fade-in animation on first render
        Self::maybe_add_popup_fade_effect(app, PopupType::Help, popup_area, theme);

        let overlay = PanelOverlay::new(" Help ")
            .border_style(theme.panel_border_style())
            .title_style(theme.panel_title_style())
            .content(lines)
            .content_bg(theme.bg_light)
            .content_style(theme.panel_content_style())
            .selected_style(theme.panel_selected_style());

        frame.render_widget(Clear, popup_area);
        frame.render_widget(overlay, popup_area);
    }

    fn render_chat_area(frame: &mut Frame, area: Rect, app: &mut App, theme: &Theme) {
        use ratatui::widgets::Block;
        use super::messages::ChatRenderer;
        frame.render_widget(Clear, area);
        // Fill full area with chat bg — Block reliably fills all cells
        let bg = Block::new().style(Style::default().bg(theme.bg));
        frame.render_widget(bg, area);
        // 1-char horizontal padding on each side
        let padded = Rect {
            x: area.x.saturating_add(1),
            y: area.y,
            width: area.width.saturating_sub(2),
            height: area.height,
        };
        if padded.width > 0 {
            ChatRenderer::render(frame, padded, app, theme);
        }
    }


    fn render_input_area(frame: &mut Frame, area: Rect, app: &App, theme: &Theme) {
        use super::editor::EditorRenderer;
        use ratatui::widgets::Block;
        frame.render_widget(Clear, area);
        // Fill with bg_light to visually distinguish input area
        let bg = Block::new().style(Style::default().bg(theme.bg_light));
        frame.render_widget(bg, area);
        // Render side borders (left and right only, no top/bottom)
        let border = Block::new()
            .borders(ratatui::widgets::Borders::LEFT | ratatui::widgets::Borders::RIGHT)
            .border_style(Style::default().fg(theme.accent).add_modifier(ratatui::style::Modifier::BOLD));
        frame.render_widget(border, area);
        // Inner area for editor content (inside borders)
        let inner = Rect {
            x: area.x.saturating_add(1),
            y: area.y,
            width: area.width.saturating_sub(2),
            height: area.height,
        };
        if inner.width > 0 {
            EditorRenderer::render(frame, inner, app, theme);
        }
    }

    /// Render the command palette overlay.
    ///
    /// Horizontally centered, anchored above the input row, grows upward.
    /// Contains: input row (with "> " prefix), accent divider, filtered command list.
    const PALETTE_MAX_VISIBLE: usize = 10;

    fn render_command_palette(
        frame: &mut Frame,
        chat_area: Rect,
        input_row: Rect,
        app: &mut App,
        theme: &Theme,
    ) {
        use super::editor::fuzzy_match_palette_commands;

        let query = app.palette_query().to_string();
        let matches = fuzzy_match_palette_commands(&query, Self::PALETTE_MAX_VISIBLE);
        let visible_count = matches.len().min(Self::PALETTE_MAX_VISIBLE);

        // Palette dimensions
        // Content rows: 1 (input) + N (commands) = N + 1
        // Block borders: 1 (top) + 1 (bottom) = 2
        // Total: N + 3
        let palette_height = (visible_count + 3) as u16;
        let palette_width = (f64::from(chat_area.width) * 0.6) as u16;
        let palette_width = palette_width.clamp(40, 100).min(chat_area.width);

        // Clamp height to available space
        let space_above = input_row.y.saturating_sub(chat_area.y);
        let palette_height = palette_height.min(space_above);
        if palette_height < 4 {
            return; // Need at least 4: top border + input + 1 command + bottom border
        }

        // Horizontal centering
        let palette_x = chat_area.x + (chat_area.width - palette_width) / 2;
        let palette_y = input_row.y - palette_height;
        let palette_y = palette_y.max(chat_area.y);

        let palette_area = Rect {
            x: palette_x,
            y: palette_y,
            width: palette_width,
            height: palette_height,
        };

        // Add sweep animation on first render
        Self::maybe_add_popup_fade_effect(app, PopupType::CommandPalette, palette_area, theme);

        // Build command list
        let commands: Vec<(String, String)> = matches
            .iter()
            .take(visible_count)
            .map(|(_, cmd)| {
                let desc = crate::commands::get_palette_command_description(cmd.id, app);
                (cmd.name.to_string(), desc)
            })
            .collect();

        // Input row prefix
        let prefix = match app.modal.state {
            super::mode::AppState::CommandPalette => "> ",
            _ => "> ",
        };

        let overlay = CommandPaletteOverlay::new()
            .query(&query)
            .commands(commands)
            .selected(app.modal.command_palette_selected)
            .scroll_offset(app.modal.command_palette_scroll_offset)
            .prefix(prefix)
            .border_style(theme.panel_border_style())
            .title_style(theme.panel_title_style())
            .content_bg(theme.bg_light)
            .text_color(theme.text)
            .accent_color(theme.accent)
            .title(" Commands ")
            .max_visible(Self::PALETTE_MAX_VISIBLE);

        frame.render_widget(Clear, palette_area);
        frame.render_widget(overlay, palette_area);
    }
}
