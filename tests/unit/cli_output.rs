//! Unit tests for CLI output formatting functions

use tui_framework::app::background_tasks::ProgressReporter;
use tui_framework::cli_output;

#[test]
fn test_format_progress() {
    // Unit test: format_progress() function
    let progress = ProgressReporter::with_message(45, 200, "Processing file.jpg");
    let formatted = cli_output::format_progress(&progress);
    assert_eq!(formatted, "[45/200] Processing file.jpg");

    // Without message
    let progress = ProgressReporter::new(45, 200);
    let formatted = cli_output::format_progress(&progress);
    assert_eq!(formatted, "[45/200]");
}

#[test]
fn test_format_progress_with_percentage() {
    // Unit test: format_progress_with_percentage() function
    let progress = ProgressReporter::with_message(45, 200, "Processing file.jpg");
    let formatted = cli_output::format_progress_with_percentage(&progress);
    assert_eq!(formatted, "[45/200] 22.5% - Processing file.jpg");

    // Without message
    let progress = ProgressReporter::new(45, 200);
    let formatted = cli_output::format_progress_with_percentage(&progress);
    assert_eq!(formatted, "[45/200] 22.5%");

    // Progress > 100%
    let progress = ProgressReporter::new(200, 150);
    let formatted = cli_output::format_progress_with_percentage(&progress);
    assert_eq!(formatted, "[200/150] 100.0%");
}

#[test]
fn test_format_progress_with_percentage_indeterminate() {
    // Unit test: format_progress_with_percentage() with indeterminate progress (None total)
    let mut progress = ProgressReporter::new(45, 0);
    progress.total = None;
    let formatted = cli_output::format_progress_with_percentage(&progress);
    // Should show count only, no percentage
    assert!(formatted.contains("45"));
    assert!(!formatted.contains("%"));

    // With message
    let mut progress = ProgressReporter::with_message(45, 0, "Processing...");
    progress.total = None;
    let formatted = cli_output::format_progress_with_percentage(&progress);
    assert!(formatted.contains("45"));
    assert!(formatted.contains("Processing..."));
    assert!(!formatted.contains("%"));
}

#[test]
fn test_formatting_with_80_character_terminal_width() {
    // Unit test: Formatting functions with 80-character terminal width to verify SC-005 (readability)
    // Test that formatted output remains readable at standard terminal width

    // Standard progress with message
    let progress = ProgressReporter::with_message(45, 200, "Processing file.jpg");
    let formatted = cli_output::format_progress_with_percentage(&progress);
    assert!(
        formatted.len() <= 80,
        "Formatted output should fit in 80 characters"
    );
    assert!(formatted.contains("45"));
    assert!(formatted.contains("200"));
    assert!(formatted.contains("22.5%"));

    // Long message should still be readable
    let progress = ProgressReporter::with_message(
        45,
        200,
        "Processing very long filename that might exceed normal width.jpg",
    );
    let formatted = cli_output::format_progress_with_percentage(&progress);
    // Even with long messages, format should be consistent
    assert!(formatted.starts_with("[45/200]"));
    assert!(formatted.contains("22.5%"));

    // Large numbers should still format correctly
    let progress = ProgressReporter::with_message(999999, 1000000, "Final item");
    let formatted = cli_output::format_progress_with_percentage(&progress);
    assert!(formatted.contains("999999"));
    assert!(formatted.contains("1000000"));
    assert!(formatted.contains("100.0%"));

    // Verify format is consistent and readable
    let progress2 = ProgressReporter::new(1, 1);
    let formatted2 = cli_output::format_progress_with_percentage(&progress2);
    assert!(formatted2.starts_with("[1/1]"));
    assert!(formatted2.contains("100.0%"));
}
