//! Foundational tests for CLI output utilities
//!
//! Tests for terminal detection, output mode detection, and formatting options.

use cli_framework::cli_output::{should_use_color, FormattingOptions, OutputMode};

#[test]
fn test_output_mode_detection() {
    // OutputMode::detect() should return Tui if stdout is TTY, Cli otherwise
    let mode = OutputMode::detect();
    // In test environment, this may vary, but the function should not panic
    assert!(matches!(mode, OutputMode::Tui | OutputMode::Cli));
}

#[test]
fn test_output_mode_default() {
    // Default should use detection
    let mode = OutputMode::default();
    assert!(matches!(mode, OutputMode::Tui | OutputMode::Cli));
}

#[test]
fn test_should_use_color() {
    // Test that should_use_color respects NO_COLOR
    // Save original value
    let original_no_color = std::env::var("NO_COLOR").ok();

    // Test with NO_COLOR set
    std::env::set_var("NO_COLOR", "1");
    assert!(
        !should_use_color(),
        "Color should be disabled when NO_COLOR is set"
    );

    // Test with NO_COLOR unset (restore original)
    if let Some(val) = original_no_color {
        std::env::set_var("NO_COLOR", val);
    } else {
        std::env::remove_var("NO_COLOR");
    }
    // Result depends on TTY, but function should not panic
    let _ = should_use_color();
}

#[test]
fn test_formatting_options_default() {
    let opts = FormattingOptions::default();
    assert!(matches!(opts.mode, OutputMode::Tui | OutputMode::Cli));
    assert_eq!(opts.json_indent, 2);
    // use_color and terminal_width depend on environment, but should be set
}

#[test]
fn test_formatting_options_custom() {
    let opts = FormattingOptions {
        mode: OutputMode::Cli,
        use_color: false,
        terminal_width: Some(100),
        json_indent: 4,
    };
    assert_eq!(opts.mode, OutputMode::Cli);
    assert!(!opts.use_color);
    assert_eq!(opts.terminal_width, Some(100));
    assert_eq!(opts.json_indent, 4);
}
