use ratatui::style::{Color, Modifier, Style};

/// gh-dark inspired color theme.
#[derive(Debug)]
pub struct Theme {
    pub bg: Color,
    pub bg_dark: Color,
    pub bg_light: Color,
    pub bg_active: Color,
    pub text: Color,
    pub text_muted: Color,
    pub text_dim: Color,
    pub border: Color,
    pub accent: Color,
    pub green: Color,
    pub red: Color,
    pub orange: Color,
    pub cyan: Color,
    pub blue: Color,
    pub blue_bright: Color,
    pub purple: Color,
}

impl Theme {
    #[must_use]
    pub fn gh_dark() -> Self {
        Self {
            bg: Color::Rgb(0x21, 0x28, 0x30),
            bg_dark: Color::Rgb(0x15, 0x1B, 0x23),
            bg_light: Color::Rgb(0x2a, 0x31, 0x3c),
            bg_active: Color::Rgb(0x26, 0x2c, 0x36),
            text: Color::Rgb(0xd1, 0xd7, 0xe0),
            text_muted: Color::Rgb(0x91, 0x98, 0xa1),
            text_dim: Color::Rgb(0x65, 0x6c, 0x76),
            border: Color::Rgb(0x3d, 0x44, 0x4d),
            accent: Color::Rgb(0x47, 0x8b, 0xe6),
            green: Color::Rgb(0x3e, 0x8a, 0x41),
            red: Color::Rgb(0xe5, 0x53, 0x4b),
            orange: Color::Rgb(0xc6, 0x90, 0x26),
            cyan: Color::Rgb(0x2a, 0x9e, 0xa8),
            blue: Color::Rgb(0x53, 0x9b, 0xf5),
            blue_bright: Color::Rgb(0x4a, 0x8e, 0xd6),
            purple: Color::Rgb(0xb0, 0x83, 0xf0),
        }
    }

    // Style builders
    #[must_use]
    pub fn bg_style(&self) -> Style {
        Style::default().bg(self.bg)
    }

    #[must_use]
    pub fn text_style(&self) -> Style {
        Style::default().fg(self.text).bg(self.bg)
    }

    #[must_use]
    pub fn text_muted_style(&self) -> Style {
        Style::default().fg(self.text_muted).bg(self.bg)
    }

    #[must_use]
    pub fn text_dim_style(&self) -> Style {
        Style::default().fg(self.text_dim).bg(self.bg)
    }

    #[must_use]
    pub fn accent_style(&self) -> Style {
        Style::default().fg(self.accent).bg(self.bg)
    }

    #[must_use]
    pub fn accent_bold_style(&self) -> Style {
        Style::default().fg(self.accent).bg(self.bg).add_modifier(Modifier::BOLD)
    }

    #[must_use]
    pub fn heading_style(&self) -> Style {
        Style::default().fg(self.blue_bright).bg(self.bg).add_modifier(Modifier::BOLD)
    }

    #[must_use]
    pub fn bold_style(&self) -> Style {
        Style::default().fg(self.text).bg(self.bg).add_modifier(Modifier::BOLD)
    }

    #[must_use]
    pub fn italic_style(&self) -> Style {
        Style::default().fg(self.text_muted).bg(self.bg).add_modifier(Modifier::ITALIC)
    }

    #[must_use]
    pub fn inline_code_style(&self) -> Style {
        Style::default().fg(self.cyan)
    }

    #[must_use]
    pub fn link_style(&self) -> Style {
        Style::default().fg(self.cyan).bg(self.bg)
    }

    #[must_use]
    pub fn error_style(&self) -> Style {
        Style::default().fg(self.red).bg(self.bg)
    }

    #[must_use]
    pub fn warning_style(&self) -> Style {
        Style::default().fg(self.orange).bg(self.bg)
    }

    #[must_use]
    pub fn success_style(&self) -> Style {
        Style::default().fg(self.green).bg(self.bg)
    }

    #[must_use]
    pub fn border_style(&self) -> Style {
        Style::default().fg(self.border)
    }

    #[must_use]
    pub fn border_accent_style(&self) -> Style {
        Style::default().fg(self.accent)
    }

    #[must_use]
    pub fn status_bar_style(&self) -> Style {
        Style::default().fg(self.text_muted).bg(self.bg)
    }

    #[must_use]
    pub fn status_accent_style(&self) -> Style {
        Style::default().fg(self.accent).bg(self.bg)
    }

    #[must_use]
    pub fn user_msg_style(&self) -> Style {
        Style::default().fg(self.text).bg(self.bg_light)
    }

    #[must_use]
    pub fn selected_style(&self) -> Style {
        Style::default().fg(self.text).bg(self.bg_active).add_modifier(Modifier::BOLD)
    }

    #[must_use]
    pub fn list_bullet_style(&self) -> Style {
        Style::default().fg(self.cyan).bg(self.bg)
    }

    #[must_use]
    pub fn code_block_style(&self) -> Style {
        Style::default().bg(self.bg_dark)
    }

    /// Inverted style for text selection highlight.
    #[must_use]
    pub fn selection_style(&self) -> Style {
        Style::default().fg(self.text).bg(self.accent).add_modifier(Modifier::BOLD)
    }

    /// Style for reasoning text body (italic, muted, distinct bg shade).
    #[must_use]
    pub fn thinking_body_style(&self) -> Style {
        Style::default()
            .fg(self.text_dim)
            .bg(self.bg)
            .add_modifier(Modifier::ITALIC)
    }

    /// Style for reasoning block header.
    #[must_use]
    pub fn thinking_header_style(&self) -> Style {
        Style::default()
            .fg(self.text_muted)
            .bg(self.bg)
            .add_modifier(Modifier::BOLD)
    }

    /// Style for reasoning block separator line.
    #[must_use]
    pub fn thinking_separator_style(&self) -> Style {
        Style::default().fg(self.border)
    }

    // Tool call styles

    /// Semantic header color for FilledHeaderBar-based tool call display.
    /// read=cyan, write=green, edit=orange, `bash=blue_bright`, error=red.
    #[must_use]
    pub fn tool_header_color(&self, tool_name: &str, has_error: bool) -> Color {
        if has_error {
            return self.red;
        }
        match tool_name {
            "read" => self.cyan,
            "write" => self.green,
            "edit" => self.orange,
            "bash" => self.blue_bright,
            _ => self.red, // unknown tools get red
        }
    }

    /// Style for the `read` tool name label (cyan, bold).
    #[must_use]
    pub fn tool_read_style(&self) -> Style {
        Style::default().fg(self.cyan).bg(self.bg_dark).add_modifier(Modifier::BOLD)
    }

    /// Style for the `write` tool name label (green, bold).
    #[must_use]
    pub fn tool_write_style(&self) -> Style {
        Style::default().fg(self.green).bg(self.bg_dark).add_modifier(Modifier::BOLD)
    }

    /// Style for the `edit` tool name label (orange, bold).
    #[must_use]
    pub fn tool_edit_style(&self) -> Style {
        Style::default().fg(self.orange).bg(self.bg_dark).add_modifier(Modifier::BOLD)
    }

    /// Style for the `bash` tool name label (purple, bold).
    #[must_use]
    pub fn tool_bash_style(&self) -> Style {
        Style::default().fg(self.purple).bg(self.bg_dark).add_modifier(Modifier::BOLD)
    }

    /// Style for error tool name label (red, bold).
    #[must_use]
    pub fn tool_error_style(&self) -> Style {
        Style::default().fg(self.red).bg(self.bg_dark).add_modifier(Modifier::BOLD)
    }

    /// Style for the target (filename or command) in a tool call header (accent, italic).
    #[must_use]
    pub fn tool_target_style(&self) -> Style {
        Style::default().fg(self.accent).bg(self.bg_dark).add_modifier(Modifier::ITALIC)
    }

    /// Style for metadata in a tool call header (muted).
    #[must_use]
    pub fn tool_meta_style(&self) -> Style {
        Style::default().fg(self.text_muted).bg(self.bg)
    }

    /// Style for content lines inside a tool call (normal text on bg).
    #[must_use]
    pub fn tool_content_style(&self) -> Style {
        Style::default().fg(self.text).bg(self.bg)
    }

    /// Style for truncated summary line ("... (N more lines, M total)").
    #[must_use]
    pub fn tool_truncated_style(&self) -> Style {
        Style::default().fg(self.text_dim).bg(self.bg)
    }

    /// Style for removed lines in edit diff (red, muted text).
    #[must_use]
    pub fn tool_diff_remove_style(&self) -> Style {
        Style::default().fg(self.red).bg(self.bg)
    }

    /// Style for added lines in edit diff (green, normal text).
    #[must_use]
    pub fn tool_diff_add_style(&self) -> Style {
        Style::default().fg(self.green).bg(self.bg)
    }

    /// Style for error output lines (red text on bg).
    #[must_use]
    pub fn tool_error_content_style(&self) -> Style {
        Style::default().fg(self.red).bg(self.bg)
    }

    // Panel styles (double-line border overlays)

    /// Style for panel borders (`blue_bright` fg on `bg_light`).
    #[must_use]
    pub fn panel_border_style(&self) -> Style {
        Style::default().fg(self.blue_bright).bg(self.bg_light)
    }

    /// Style for panel title bar (reversed: dark text on bright blue).
    ///
    /// fg + bg are set "backwards" so that REVERSED produces the final look:
    /// `fg=bg_light` (dark text), `bg=blue_bright` (bright bar, fills title row).
    #[must_use]
    pub fn panel_title_style(&self) -> Style {
        Style::default()
            .fg(self.blue_bright)
            .bg(self.bg_light)
            .add_modifier(Modifier::BOLD)
            .add_modifier(Modifier::REVERSED)
    }

    /// Style for panel content area (text on `bg_light`).
    #[must_use]
    pub fn panel_content_style(&self) -> Style {
        Style::default().fg(self.text).bg(self.bg_light)
    }

    /// Style for selected item inside a panel (text on accent, bold).
    #[must_use]
    pub fn panel_selected_style(&self) -> Style {
        Style::default().fg(self.text).bg(self.accent).add_modifier(Modifier::BOLD)
    }
}

// ─── ratatui-md integration ───

impl ratatui_md::MdTheme for Theme {
    // Parsing
    fn text_style(&self) -> Style {
        Theme::text_style(self)
    }
    fn heading_style(&self) -> Style {
        Theme::heading_style(self)
    }
    fn text_muted_color(&self) -> Color {
        self.text_muted
    }
    fn link_style(&self) -> Style {
        Theme::link_style(self)
    }
    fn inline_code_style(&self) -> Style {
        Theme::inline_code_style(self)
    }

    // Rendering
    fn list_bullet_style(&self) -> Style {
        Theme::list_bullet_style(self)
    }
    fn code_block_style(&self) -> Style {
        Theme::code_block_style(self)
    }
    fn text_dim_color(&self) -> Color {
        self.text_dim
    }
    fn text_muted_style(&self) -> Style {
        Theme::text_muted_style(self)
    }
}
