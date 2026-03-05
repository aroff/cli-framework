//! CLI Output Utilities
//!
//! This module provides utilities for formatting and displaying structured output
//! in CLI applications, including tables, JSON, progress indicators, and formatted messages.
//!
//! This module is being implemented as part of feature 006-cli-output-utilities.
//! See `specs/006-cli-output-utilities/` for design documents.

use crossterm::tty::IsTty;

/// Output mode for CLI formatting
///
/// Determines how output should be formatted based on terminal capabilities.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutputMode {
    /// Interactive terminal mode (TUI)
    /// Truncates columns with ellipsis when width exceeded, allows horizontal scrolling
    Tui,
    /// Non-interactive CLI mode
    /// Outputs entire content without truncation for piping/scripting
    Cli,
}

impl Default for OutputMode {
    fn default() -> Self {
        Self::detect()
    }
}

impl OutputMode {
    /// Detect output mode from terminal capabilities
    ///
    /// Returns `Tui` if stdout is a TTY, `Cli` otherwise.
    pub fn detect() -> Self {
        if std::io::stdout().is_tty() {
            OutputMode::Tui
        } else {
            OutputMode::Cli
        }
    }
}

/// Formatting options for CLI output
///
/// Configuration options that control how output is formatted.
#[derive(Debug, Clone)]
pub struct FormattingOptions {
    /// Output mode (TUI or CLI)
    pub mode: OutputMode,
    /// Whether to use color (respects NO_COLOR environment variable)
    pub use_color: bool,
    /// Terminal width for wrapping/truncation (None = auto-detect)
    pub terminal_width: Option<usize>,
    /// JSON indentation spaces (default: 2)
    pub json_indent: usize,
}

impl Default for FormattingOptions {
    fn default() -> Self {
        Self {
            mode: OutputMode::detect(),
            use_color: should_use_color(),
            terminal_width: None, // Will be detected when needed
            json_indent: 2,
        }
    }
}

/// Check if color output should be used
///
/// Returns `true` if colors should be enabled for stdout output.
///
/// This function uses `cli_mode::should_color_output()` which respects:
/// - NO_COLOR environment variable (highest precedence)
/// - FORCE_COLOR environment variable
/// - TTY detection (stdout is a TTY)
///
/// # Examples
///
/// ```no_run
/// use cli_framework::cli_output;
///
/// if cli_output::should_use_color() {
///     println!("\x1b[32mColored text\x1b[0m");
/// }
/// ```
pub fn should_use_color() -> bool {
    crate::cli_mode::should_color_output()
}

pub mod ask;
pub mod json;
pub mod message;
#[cfg(feature = "progress")]
pub mod progress;
pub mod table;

// Re-export commonly used types
pub use ask::*;
pub use json::{format_json, format_json_compact, print_json};
pub use message::{format_message, format_message_with_details, print_message};
#[cfg(feature = "progress")]
pub use progress::create_progress_bar;
pub use table::{format_table, print_table, Alignment, ColumnDef, GridData};
