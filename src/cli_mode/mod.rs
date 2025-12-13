//! CLI Mode Detection
//!
//! This module provides utilities for detecting CLI execution context and adapting
//! application behavior accordingly. Applications can automatically detect whether
//! they are running in an interactive terminal, determine output preferences (colors,
//! format), and adapt behavior for interactive vs non-interactive environments.
//!
//! # Features
//!
//! - **Terminal Detection**: Check if stdin/stdout/stderr are connected to terminals (TTY)
//! - **Interactive Mode**: Detect if the application can interact with the user
//! - **Color Output**: Determine if colors should be enabled based on terminal and environment
//! - **Output Format**: Select appropriate output format (table, JSON, plain) based on context
//! - **Progress Indicators**: Determine if progress indicators should be shown
//! - **Quiet Mode**: Detect if the application is running in quiet mode
//! - **Terminal Dimensions**: Get terminal width and height when available
//!
//! # Environment Variables
//!
//! The module respects standard environment variables:
//!
//! - `NO_COLOR`: Disables color output (highest precedence for colors)
//! - `FORCE_COLOR`: Forces color output (only when NO_COLOR is not set)
//! - `OUTPUT_FORMAT`: Sets output format preference (table, json, plain)
//! - `QUIET`: Enables quiet mode (suppresses progress indicators and non-essential output)
//! - `COLUMNS`: Terminal width in characters (for terminal_width())
//! - `ROWS`: Terminal height in characters (for terminal_height())
//!
//! # Examples
//!
//! ## Basic Usage
//!
//! ```no_run
//! use tui_framework::cli_mode;
//!
//! // Check if running in interactive terminal
//! if cli_mode::is_interactive() {
//!     println!("Running in interactive mode");
//!     // Show prompts, progress bars, etc.
//! } else {
//!     println!("Running in batch mode");
//!     // Use JSON output, no prompts
//! }
//!
//! // Check if colors should be enabled
//! if cli_mode::should_color_output() {
//!     println!("\x1b[32mColored output\x1b[0m");
//! } else {
//!     println!("Plain text output");
//! }
//!
//! // Get preferred output format
//! let format = cli_mode::get_output_format();
//! match format {
//!     cli_mode::OutputFormat::Table => {
//!         // Render as table for human readability
//!     }
//!     cli_mode::OutputFormat::Json => {
//!         // Output JSON for scripting
//!     }
//!     cli_mode::OutputFormat::Plain => {
//!         // Output plain text
//!     }
//! }
//! ```
//!
//! ## Stream-Specific Detection
//!
//! ```no_run
//! use tui_framework::cli_mode;
//!
//! // Check specific streams independently
//! if cli_mode::is_stdout_tty() {
//!     // stdout is connected to terminal
//! }
//!
//! if cli_mode::is_stderr_tty() {
//!     // stderr is connected to terminal
//!     // Can color stderr differently from stdout
//! }
//!
//! // Color detection for specific streams
//! if cli_mode::should_color_output() {
//!     // Colors for stdout
//! }
//!
//! if cli_mode::should_color_stderr() {
//!     // Colors for stderr
//! }
//! ```
//!
//! ## Progress Indicators
//!
//! ```no_run
//! use tui_framework::cli_mode;
//!
//! if cli_mode::should_show_progress() {
//!     // Show progress bar
//!     println!("Processing... [████████░░] 80%");
//! } else {
//!     // Suppress progress indicators
//! }
//! ```
//!
//! ## Terminal Dimensions
//!
//! ```no_run
//! use tui_framework::cli_mode;
//!
//! if let Some(width) = cli_mode::terminal_width() {
//!     println!("Terminal width: {} characters", width);
//!     // Adjust table width accordingly
//! }
//!
//! if let Some(height) = cli_mode::terminal_height() {
//!     println!("Terminal height: {} characters", height);
//!     // Adjust pagination accordingly
//! }
//! ```

use std::io::{self, IsTerminal};
use std::panic;

/// Check if stdout is connected to a terminal (TTY)
///
/// Returns `true` if stdout is connected to an interactive terminal,
/// `false` otherwise (e.g., when output is piped or redirected).
///
/// # Examples
///
/// ```no_run
/// use tui_framework::cli_mode;
///
/// if cli_mode::is_stdout_tty() {
///     println!("Outputting to terminal");
/// } else {
///     println!("Output is piped or redirected");
/// }
/// ```
pub fn is_stdout_tty() -> bool {
    safe_tty_check(|| io::stdout().is_terminal())
}

/// Check if stderr is connected to a terminal (TTY)
///
/// Returns `true` if stderr is connected to an interactive terminal,
/// `false` otherwise (e.g., when stderr is piped or redirected).
///
/// # Examples
///
/// ```no_run
/// use tui_framework::cli_mode;
///
/// if cli_mode::is_stderr_tty() {
///     eprintln!("Error outputting to terminal");
/// } else {
///     eprintln!("Error output is piped or redirected");
/// }
/// ```
pub fn is_stderr_tty() -> bool {
    safe_tty_check(|| io::stderr().is_terminal())
}

/// Check if stdin is connected to a terminal (TTY)
///
/// Returns `true` if stdin is connected to an interactive terminal,
/// `false` otherwise (e.g., when input is piped or redirected).
///
/// # Examples
///
/// ```no_run
/// use tui_framework::cli_mode;
///
/// if cli_mode::is_stdin_tty() {
///     println!("Reading from terminal");
/// } else {
///     println!("Input is piped or redirected");
/// }
/// ```
pub fn is_stdin_tty() -> bool {
    safe_tty_check(|| io::stdin().is_terminal())
}

/// Safe TTY detection wrapper that catches panics and returns false (non-interactive default)
///
/// This function wraps TTY detection calls to ensure that any panics or errors
/// result in a safe default (non-interactive mode) rather than propagating errors.
///
/// # Safety
///
/// Uses `std::panic::catch_unwind` to catch any panics from TTY detection.
/// Returns `false` (non-interactive default) if detection fails or panics.
fn safe_tty_check<F>(check: F) -> bool
where
    F: FnOnce() -> bool + panic::UnwindSafe,
{
    panic::catch_unwind(check).unwrap_or(false)
}

/// Read environment variable with case-insensitive support
///
/// Reads an environment variable and returns its value as a lowercase string.
/// Returns `None` if the variable is not set.
///
/// # Examples
///
/// ```no_run
/// use tui_framework::cli_mode;
///
/// if let Some(value) = cli_mode::read_env_var("OUTPUT_FORMAT") {
///     println!("OUTPUT_FORMAT is set to: {}", value);
/// }
/// ```
pub fn read_env_var(name: &str) -> Option<String> {
    std::env::var(name)
        .ok()
        .map(|v| v.to_lowercase().trim().to_string())
}

/// Check if NO_COLOR environment variable is set
///
/// Returns `true` if NO_COLOR is set (any value), `false` otherwise.
/// This follows the NO_COLOR standard: if the variable is set, colors should be disabled.
///
/// # Examples
///
/// ```no_run
/// use tui_framework::cli_mode;
///
/// if cli_mode::is_no_color_set() {
///     println!("Colors disabled by NO_COLOR");
/// }
/// ```
pub fn is_no_color_set() -> bool {
    std::env::var("NO_COLOR").is_ok()
}

/// Check if FORCE_COLOR environment variable is set
///
/// Returns `true` if FORCE_COLOR is set (any value), `false` otherwise.
/// Note: FORCE_COLOR only applies when NO_COLOR is not set.
///
/// # Examples
///
/// ```no_run
/// use tui_framework::cli_mode;
///
/// if cli_mode::is_force_color_set() {
///     println!("Colors forced by FORCE_COLOR");
/// }
/// ```
pub fn is_force_color_set() -> bool {
    std::env::var("FORCE_COLOR").is_ok()
}

/// Check if colors should be enabled for stdout
///
/// Returns `true` if colors should be enabled for stdout output, `false` otherwise.
///
/// Precedence order (highest to lowest):
/// 1. NO_COLOR environment variable (if set, colors are disabled)
/// 2. FORCE_COLOR environment variable (if set and NO_COLOR is not set, colors are enabled)
/// 3. TTY detection (if stdout is a TTY, colors are enabled by default)
///
/// # Examples
///
/// ```no_run
/// use tui_framework::cli_mode;
///
/// if cli_mode::should_color_output() {
///     println!("\x1b[32mGreen text\x1b[0m");
/// } else {
///     println!("Plain text");
/// }
/// ```
pub fn should_color_output() -> bool {
    // NO_COLOR takes highest precedence
    if is_no_color_set() {
        return false;
    }

    // FORCE_COLOR can override TTY detection
    if is_force_color_set() {
        return true;
    }

    // Default: check if stdout is a TTY
    is_stdout_tty()
}

/// Check if colors should be enabled for stderr
///
/// Returns `true` if colors should be enabled for stderr output, `false` otherwise.
///
/// Precedence order (highest to lowest):
/// 1. NO_COLOR environment variable (if set, colors are disabled)
/// 2. FORCE_COLOR environment variable (if set and NO_COLOR is not set, colors are enabled)
/// 3. TTY detection (if stderr is a TTY, colors are enabled by default)
///
/// # Examples
///
/// ```no_run
/// use tui_framework::cli_mode;
///
/// if cli_mode::should_color_stderr() {
///     eprintln!("\x1b[31mRed error text\x1b[0m");
/// } else {
///     eprintln!("Plain error text");
/// }
/// ```
pub fn should_color_stderr() -> bool {
    // NO_COLOR takes highest precedence
    if is_no_color_set() {
        return false;
    }

    // FORCE_COLOR can override TTY detection
    if is_force_color_set() {
        return true;
    }

    // Default: check if stderr is a TTY
    is_stderr_tty()
}

/// Check if the application is running in interactive mode
///
/// Returns `true` if both stdin and stdout are connected to terminals,
/// `false` otherwise. Interactive mode means the application can
/// prompt for user input and display interactive output.
///
/// # Examples
///
/// ```no_run
/// use tui_framework::cli_mode;
///
/// if cli_mode::is_interactive() {
///     // Show prompts, progress bars, etc.
/// } else {
///     // Use JSON output, no prompts
/// }
/// ```
pub fn is_interactive() -> bool {
    // Both stdin and stdout must be TTYs for interactive mode
    is_stdin_tty() && is_stdout_tty()
}

/// Check if the application is running in quiet mode
///
/// Returns `true` if QUIET environment variable is set, `false` otherwise.
/// Quiet mode suppresses non-essential output and progress indicators.
///
/// # Examples
///
/// ```no_run
/// use tui_framework::cli_mode;
///
/// if !cli_mode::is_quiet() {
///     println!("Verbose output");
/// }
/// ```
pub fn is_quiet() -> bool {
    std::env::var("QUIET").is_ok()
}

/// Check if progress indicators should be shown
///
/// Returns `true` if progress indicators should be displayed, `false` otherwise.
///
/// Progress indicators are shown when:
/// - Running in an interactive terminal (stdout is TTY)
/// - Not in quiet mode (QUIET environment variable is not set)
///
/// # Examples
///
/// ```no_run
/// use tui_framework::cli_mode;
///
/// if cli_mode::should_show_progress() {
///     // Show progress bar
/// } else {
///     // Suppress progress indicators
/// }
/// ```
pub fn should_show_progress() -> bool {
    // Progress indicators require interactive terminal and not quiet mode
    is_stdout_tty() && !is_quiet()
}

/// Output format preference
///
/// Determines how output should be formatted based on execution environment
/// and user preferences.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutputFormat {
    /// Human-readable table format (for interactive terminals)
    Table,
    /// JSON format (for scripting and automation)
    Json,
    /// Plain text format
    Plain,
}

impl OutputFormat {
    /// Detect format from OUTPUT_FORMAT environment variable
    ///
    /// Returns `Some(OutputFormat)` if OUTPUT_FORMAT is set to a valid value,
    /// `None` otherwise.
    ///
    /// Valid values (case-insensitive): "table", "json", "plain"
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use tui_framework::cli_mode;
    ///
    /// if let Some(format) = cli_mode::OutputFormat::from_env() {
    ///     println!("Using format: {:?}", format);
    /// }
    /// ```
    pub fn from_env() -> Option<Self> {
        read_env_var("OUTPUT_FORMAT").and_then(|s| match s.as_str() {
            "table" => Some(OutputFormat::Table),
            "json" => Some(OutputFormat::Json),
            "plain" => Some(OutputFormat::Plain),
            _ => None,
        })
    }

    /// Get default format based on TTY status
    ///
    /// Returns `Table` for interactive terminals (stdout is TTY),
    /// `Json` for non-interactive environments (stdout is not TTY).
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use tui_framework::cli_mode;
    ///
    /// let format = cli_mode::OutputFormat::default_for_tty();
    /// ```
    pub fn default_for_tty() -> Self {
        if is_stdout_tty() {
            OutputFormat::Table
        } else {
            OutputFormat::Json
        }
    }
}

/// Get preferred output format
///
/// Returns the preferred output format based on environment variables
/// and terminal detection.
///
/// Precedence order:
/// 1. OUTPUT_FORMAT environment variable (if set to valid value)
/// 2. Terminal-based default (Table for TTY, Json for non-TTY)
///
/// # Examples
///
/// ```no_run
/// use tui_framework::cli_mode;
///
/// let format = cli_mode::get_output_format();
/// match format {
///     cli_mode::OutputFormat::Table => println!("Using table format"),
///     cli_mode::OutputFormat::Json => println!("Using JSON format"),
///     cli_mode::OutputFormat::Plain => println!("Using plain format"),
/// }
/// ```
pub fn get_output_format() -> OutputFormat {
    // OUTPUT_FORMAT env var takes precedence over terminal-based detection
    OutputFormat::from_env().unwrap_or_else(OutputFormat::default_for_tty)
}

/// Get terminal width in characters
///
/// Returns `Some(width)` if the terminal width can be determined,
/// `None` otherwise. Attempts to read from COLUMNS environment variable
/// first, then falls back to terminal detection if available.
///
/// # Examples
///
/// ```no_run
/// use tui_framework::cli_mode;
///
/// if let Some(width) = cli_mode::terminal_width() {
///     println!("Terminal width: {} characters", width);
/// }
/// ```
pub fn terminal_width() -> Option<usize> {
    if !is_stdout_tty() {
        return None;
    }

    // Try COLUMNS environment variable first
    if let Some(cols) = read_env_var("COLUMNS") {
        if let Ok(width) = cols.parse::<usize>() {
            return Some(width);
        }
    }

    // Terminal size detection would go here if terminal_size crate is used
    // For now, return None if COLUMNS is not available
    None
}

/// Get terminal height in characters
///
/// Returns `Some(height)` if the terminal height can be determined,
/// `None` otherwise. Attempts to read from ROWS environment variable
/// first, then falls back to terminal detection if available.
///
/// # Examples
///
/// ```no_run
/// use tui_framework::cli_mode;
///
/// if let Some(height) = cli_mode::terminal_height() {
///     println!("Terminal height: {} characters", height);
/// }
/// ```
pub fn terminal_height() -> Option<usize> {
    if !is_stdout_tty() {
        return None;
    }

    // Try ROWS environment variable first
    if let Some(rows) = read_env_var("ROWS") {
        if let Ok(height) = rows.parse::<usize>() {
            return Some(height);
        }
    }

    // Terminal size detection would go here if terminal_size crate is used
    // For now, return None if ROWS is not available
    None
}
