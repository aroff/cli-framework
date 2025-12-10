//! CLI output formatting and display utilities for progress reporting
//!
//! This module provides functions for formatting and displaying progress information
//! in terminal output, including in-place updates and final summaries.

use crate::app::background_tasks::ProgressReporter;

/// Format progress for CLI output without percentage
///
/// # Arguments
///
/// * `progress` - The progress reporter to format
///
/// # Returns
///
/// Formatted string like `"[45/200] Processing file.jpg"` or `"[45/200]"` if no message
///
/// # Example
///
/// ```rust
/// use tui_framework::app::background_tasks::ProgressReporter;
/// use tui_framework::cli_output;
///
/// let progress = ProgressReporter::with_message(45, 200, "Processing file.jpg");
/// assert_eq!(
///     cli_output::format_progress(&progress),
///     "[45/200] Processing file.jpg"
/// );
/// ```
pub fn format_progress(progress: &ProgressReporter) -> String {
    let base = if let Some(total) = progress.total {
        format!("[{}/{}]", progress.current, total)
    } else {
        format!("[{}]", progress.current)
    };

    if let Some(ref msg) = progress.message {
        format!("{} {}", base, msg)
    } else {
        base
    }
}

/// Format progress with percentage
///
/// # Arguments
///
/// * `progress` - The progress reporter to format
///
/// # Returns
///
/// * Formatted string like `"[45/200] 22.5% - Processing file.jpg"` when total is available
/// * Formatted string like `"[45] Processing file.jpg"` when total is None (indeterminate)
///
/// # Example
///
/// ```rust
/// use tui_framework::app::background_tasks::ProgressReporter;
/// use tui_framework::cli_output;
///
/// let progress = ProgressReporter::with_message(45, 200, "Processing file.jpg");
/// assert_eq!(
///     cli_output::format_progress_with_percentage(&progress),
///     "[45/200] 22.5% - Processing file.jpg"
/// );
/// ```
pub fn format_progress_with_percentage(progress: &ProgressReporter) -> String {
    let base = if let Some(total) = progress.total {
        let percentage = progress.percentage();
        format!("[{}/{}] {:.1}%", progress.current, total, percentage)
    } else {
        // Indeterminate progress: count-only, no percentage
        format!("[{}]", progress.current)
    };

    if let Some(ref msg) = progress.message {
        format!("{} - {}", base, msg)
    } else {
        base
    }
}

/// Print progress update in-place (overwrites current line)
///
/// Formats progress with percentage, prints with `\r` (carriage return) to overwrite
/// the current line, and flushes stdout to ensure immediate display.
///
/// # Arguments
///
/// * `progress` - The progress reporter to display
///
/// # Example
///
/// ```rust
/// use tui_framework::app::background_tasks::ProgressReporter;
/// use tui_framework::cli_output;
///
/// let progress = ProgressReporter::with_message(45, 200, "Processing file.jpg");
/// cli_output::print_progress_update(&progress);
/// // Terminal shows: "[45/200] 22.5% - Processing file.jpg" (overwrites previous line)
/// ```
pub fn print_progress_update(progress: &ProgressReporter) {
    let formatted = format_progress_with_percentage(progress);
    print!("\r{}", formatted);
    use std::io::Write;
    let _ = std::io::stdout().flush();
}

/// Print final progress line with newline
///
/// Formats progress with percentage, prints with `\r` and newline to preserve
/// the progress line, and flushes stdout.
///
/// # Arguments
///
/// * `progress` - The progress reporter to display
///
/// # Example
///
/// ```rust
/// use tui_framework::app::background_tasks::ProgressReporter;
/// use tui_framework::cli_output;
///
/// let progress = ProgressReporter::new(200, 200);
/// cli_output::print_progress_complete(&progress);
/// // Terminal shows: "[200/200] 100.0%\n" (newline added)
/// ```
pub fn print_progress_complete(progress: &ProgressReporter) {
    let formatted = format_progress_with_percentage(progress);
    println!("\r{}", formatted);
    use std::io::Write;
    let _ = std::io::stdout().flush();
}

/// Helper function to filter backwards progress updates (progress only moves forward)
///
/// # Arguments
///
/// * `current` - Current item count from new progress update
/// * `last_displayed` - Last displayed item count
///
/// # Returns
///
/// * `true` if `current >= last_displayed` (progress moved forward or stayed same)
/// * `false` if `current < last_displayed` (backwards update, should be ignored)
///
/// # Example
///
/// ```rust
/// use tui_framework::cli_output;
///
/// let mut last_displayed = 0;
///
/// // In your event loop:
/// // while let Ok(progress) = progress_rx.try_recv() {
/// //     if cli_output::should_display_progress(progress.current, last_displayed) {
/// //         cli_output::print_progress_update(&progress);
/// //         last_displayed = progress.current;
/// //     }
/// // }
/// ```
pub fn should_display_progress(current: usize, last_displayed: usize) -> bool {
    current >= last_displayed
}
