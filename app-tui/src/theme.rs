//! Design tokens shared by every widget. Semantic, not literal.
//!
//! Deliberate rule not needed by `app/`/`app-egui/`: don't paint a
//! background color across empty space. Those two own their whole window
//! and pick a background color; a TUI shares the user's terminal, which
//! already has one. Painting over it is what makes homegrown TUIs look
//! wrong instead of native (contrast `btop`/`lazygit`, which rely on
//! borders and accent color for structure, not fills). Background fill is
//! reserved for the selected row, the active input, and modal surfaces.

use ratatui::style::{Color, Modifier, Style};

#[derive(Debug, Clone, Copy)]
pub struct Theme {
    pub fg: Color,
    pub fg_subtle: Color,
    pub accent: Color,
    pub accent_fg: Color,
    pub border: Color,
    pub border_focus: Color,
    pub success: Color,
    pub warning: Color,
    pub danger: Color,
    pub info: Color,
    pub disabled_fg: Color,
    pub selection_bg: Color,
    pub reduced_color: bool,
}

impl Theme {
    /// Full 256/24-bit color palette - the default for any modern terminal.
    pub fn default_palette() -> Self {
        Self {
            fg: Color::Reset,
            fg_subtle: Color::DarkGray,
            accent: Color::Cyan,
            accent_fg: Color::Black,
            border: Color::DarkGray,
            border_focus: Color::Cyan,
            success: Color::Green,
            warning: Color::Yellow,
            danger: Color::Red,
            info: Color::Blue,
            disabled_fg: Color::DarkGray,
            selection_bg: Color::DarkGray,
            reduced_color: false,
        }
    }

    /// Reduced ANSI-16 fallback for limited-color terminals (or an
    /// explicit opt-in) - every color here is a base ANSI color, supported
    /// by every terminal this app targets, including legacy Windows
    /// ConHost.
    pub fn reduced_palette() -> Self {
        Self {
            fg: Color::Reset,
            fg_subtle: Color::Gray,
            accent: Color::Cyan,
            accent_fg: Color::Black,
            border: Color::Gray,
            border_focus: Color::Cyan,
            success: Color::Green,
            warning: Color::Yellow,
            danger: Color::Red,
            info: Color::Blue,
            disabled_fg: Color::Gray,
            selection_bg: Color::Gray,
            reduced_color: true,
        }
    }

    /// Honors the `NO_COLOR` convention (<https://no-color.org/>) - a
    /// signal neither existing GUI head has an equivalent for, since they
    /// always own a real window rather than sharing a terminal.
    pub fn detect() -> Self {
        if std::env::var_os("NO_COLOR").is_some() {
            Self::reduced_palette()
        } else {
            Self::default_palette()
        }
    }

    pub fn border_style(&self, focused: bool) -> Style {
        if focused {
            Style::default().fg(self.border_focus).add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(self.border)
        }
    }

    pub fn title_style(&self, focused: bool) -> Style {
        if focused {
            Style::default().fg(self.accent).add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(self.fg_subtle)
        }
    }

    /// `REVERSED` rather than a custom highlight color - works identically
    /// at every color depth, unlike a specific background color choice.
    pub fn selected_row_style(&self) -> Style {
        Style::default().add_modifier(Modifier::REVERSED)
    }

    pub fn status_style(&self, tone: StatusTone) -> Style {
        let color = match tone {
            StatusTone::Neutral => self.fg,
            StatusTone::Success => self.success,
            StatusTone::Warning => self.warning,
            StatusTone::Danger => self.danger,
            StatusTone::Info => self.info,
        };
        Style::default().fg(color)
    }

    pub fn disabled_style(&self) -> Style {
        Style::default().fg(self.disabled_fg)
    }
}

impl Default for Theme {
    fn default() -> Self {
        Self::detect()
    }
}

/// Semantic color intent - the one mapping used everywhere (status bar,
/// inline validation, notifications, gauge color) instead of scattering ad
/// hoc colors through widgets. Matches how both existing GUI heads get by
/// with one shared status-text style.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StatusTone {
    Neutral,
    Success,
    Warning,
    Danger,
    Info,
}
