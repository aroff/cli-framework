//! Theme struct with default ANSI colors
//!
//! Centralizes styling definitions (colors, modifiers) to ensure consistent UI
//! across views and standard widgets. Uses standard ANSI 16-color palette.

use ratatui::style::{Color, Modifier, Style};

/// Theme for the TUI framework
///
/// Provides default styling using standard ANSI colors (16-color palette)
/// to respect terminal user preferences and ensure readability across
/// different terminal color schemes.
#[derive(Debug, Clone)]
pub struct Theme {
    /// Style for focused/selected items
    pub primary_style: Style,
    /// Style for normal text
    pub secondary_style: Style,
    /// Style for error messages/states
    pub error_style: Style,
    /// Style for the bottom status bar
    pub status_bar_style: Style,
    /// Style for modal borders/backgrounds
    pub modal_style: Style,
}

impl Default for Theme {
    fn default() -> Self {
        Self {
            primary_style: Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
            secondary_style: Style::default().fg(Color::White),
            error_style: Style::default()
                .fg(Color::Red)
                .add_modifier(Modifier::BOLD),
            status_bar_style: Style::default()
                .fg(Color::Black)
                .bg(Color::White),
            modal_style: Style::default()
                .fg(Color::White)
                .bg(Color::Blue),
        }
    }
}

impl Theme {
    /// Create a new theme with default ANSI colors
    pub fn new() -> Self {
        Self::default()
    }

    /// Create a custom theme (allows applications to override defaults)
    pub fn custom(
        primary: Style,
        secondary: Style,
        error: Style,
        status_bar: Style,
        modal: Style,
    ) -> Self {
        Self {
            primary_style: primary,
            secondary_style: secondary,
            error_style: error,
            status_bar_style: status_bar,
            modal_style: modal,
        }
    }
}

