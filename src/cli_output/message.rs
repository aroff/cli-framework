//! Message formatting utilities
//!
//! Provides functions for formatting messages with appropriate severity indicators
//! and routing to correct output streams.

use crate::cli_output::should_use_color;
use crate::message::{AppMessage, AppMessageKind};

/// Format a message with appropriate visual indicator based on severity
///
/// # Arguments
///
/// * `msg` - Message with severity and text
///
/// # Returns
///
/// Formatted message with text prefix and optional color (FR-003, FR-003a)
pub fn format_message(msg: &AppMessage) -> String {
    let prefix = match msg.kind {
        AppMessageKind::Info => "ℹ",
        AppMessageKind::Warning => "⚠",
        AppMessageKind::Error => "✗",
    };

    let mut result = format!("{} {}", prefix, msg.short);

    // Add optional color if enabled (FR-006)
    if should_use_color() {
        let color_code = match msg.kind {
            AppMessageKind::Info => "\x1b[36m",    // Cyan
            AppMessageKind::Warning => "\x1b[33m", // Yellow
            AppMessageKind::Error => "\x1b[31m",   // Red
        };
        let reset = "\x1b[0m";
        result = format!("{}{}{}", color_code, result, reset);
    }

    result
}

/// Format a message with detailed information if available
///
/// # Arguments
///
/// * `msg` - Message with optional details
///
/// # Returns
///
/// Formatted message with details included (FR-010)
pub fn format_message_with_details(msg: &AppMessage) -> String {
    let mut result = format_message(msg);

    if let Some(ref details) = msg.details {
        result.push('\n');
        result.push_str(details);
    }

    result
}

/// Print a message to the appropriate stream (stdout for info, stderr for warnings/errors)
///
/// # Arguments
///
/// * `msg` - Message to print
///
/// # Behavior
///
/// - Info messages → stdout (FR-004)
/// - Warning/Error messages → stderr (FR-004)
pub fn print_message(msg: &AppMessage) {
    let formatted = format_message(msg);

    match msg.kind {
        AppMessageKind::Info => {
            println!("{}", formatted);
        }
        AppMessageKind::Warning | AppMessageKind::Error => {
            eprintln!("{}", formatted);
        }
    }
}
