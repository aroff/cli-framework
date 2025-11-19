//! Unit tests for Theme
//!
//! Verifies that Theme uses standard ANSI 16-color palette and fallback behavior

use tui_framework::view::Theme;
use ratatui::style::{Color, Modifier, Style};

#[test]
fn test_theme_default_uses_ansi_colors() {
    let theme = Theme::default();
    
    // Verify primary style uses ANSI colors
    assert!(matches!(theme.primary_style.fg, Some(Color::Cyan)));
    assert!(theme.primary_style.add_modifier.contains(Modifier::BOLD));
    
    // Verify secondary style uses ANSI colors
    assert!(matches!(theme.secondary_style.fg, Some(Color::White)));
    
    // Verify error style uses ANSI colors
    assert!(matches!(theme.error_style.fg, Some(Color::Red)));
    assert!(theme.error_style.add_modifier.contains(Modifier::BOLD));
    
    // Verify status bar style uses ANSI colors
    assert!(matches!(theme.status_bar_style.fg, Some(Color::Black)));
    assert!(matches!(theme.status_bar_style.bg, Some(Color::White)));
    
    // Verify modal style uses ANSI colors
    assert!(matches!(theme.modal_style.fg, Some(Color::White)));
    assert!(matches!(theme.modal_style.bg, Some(Color::Blue)));
}

#[test]
fn test_theme_new_creates_default() {
    let theme1 = Theme::new();
    let theme2 = Theme::default();
    
    // Both should have the same default colors
    assert!(matches!(theme1.primary_style.fg, Some(Color::Cyan)));
    assert!(matches!(theme2.primary_style.fg, Some(Color::Cyan)));
}

#[test]
fn test_theme_custom_allows_override() {
    let custom_primary = Style::default().fg(Color::Green);
    let custom_secondary = Style::default().fg(Color::Yellow);
    let custom_error = Style::default().fg(Color::Magenta);
    let custom_status = Style::default().fg(Color::Blue);
    let custom_modal = Style::default().fg(Color::Red);
    
    let theme = Theme::custom(
        custom_primary,
        custom_secondary,
        custom_error,
        custom_status,
        custom_modal,
    );
    
    // Verify custom colors are used
    assert!(matches!(theme.primary_style.fg, Some(Color::Green)));
    assert!(matches!(theme.secondary_style.fg, Some(Color::Yellow)));
    assert!(matches!(theme.error_style.fg, Some(Color::Magenta)));
    assert!(matches!(theme.status_bar_style.fg, Some(Color::Blue)));
    assert!(matches!(theme.modal_style.fg, Some(Color::Red)));
}

#[test]
fn test_theme_ansi_color_compatibility() {
    // Verify all colors used are from standard ANSI 16-color palette
    let theme = Theme::default();
    
    // ANSI 16-color palette includes: Black, Red, Green, Yellow, Blue, Magenta, Cyan, White
    // and their bright variants
    
    // Check that all colors are from the standard palette
    let colors = vec![
        theme.primary_style.fg,
        theme.secondary_style.fg,
        theme.error_style.fg,
        theme.status_bar_style.fg,
        theme.status_bar_style.bg,
        theme.modal_style.fg,
        theme.modal_style.bg,
    ];
    
    for color in colors {
        if let Some(c) = color {
            match c {
                Color::Black | Color::Red | Color::Green | Color::Yellow |
                Color::Blue | Color::Magenta | Color::Cyan | Color::White |
                Color::Gray | Color::LightRed | Color::LightGreen | Color::LightYellow |
                Color::LightBlue | Color::LightMagenta | Color::LightCyan | Color::White => {
                    // Valid ANSI color
                }
                _ => {
                    // This would be a non-ANSI color (like Rgb), which we want to avoid
                    panic!("Theme uses non-ANSI color: {:?}", c);
                }
            }
        }
    }
}

