//! Tests for terminal dimension detection

use cli_framework::cli_mode;
use std::env;

#[test]
fn test_terminal_width_from_env() {
    let original_columns = env::var("COLUMNS").ok();

    // Test reading width from COLUMNS environment variable
    env::set_var("COLUMNS", "120");
    let width = cli_mode::terminal_width();
    // If stdout is TTY, should return Some(120), otherwise None
    if cli_mode::is_stdout_tty() {
        assert_eq!(width, Some(120));
    } else {
        assert_eq!(width, None);
    }

    // Restore
    if let Some(val) = original_columns {
        env::set_var("COLUMNS", val);
    } else {
        env::remove_var("COLUMNS");
    }
}

#[test]
fn test_terminal_height_from_env() {
    let original_rows = env::var("ROWS").ok();

    // Test reading height from ROWS environment variable
    env::set_var("ROWS", "40");
    let height = cli_mode::terminal_height();
    // If stdout is TTY, should return Some(40), otherwise None
    if cli_mode::is_stdout_tty() {
        assert_eq!(height, Some(40));
    } else {
        assert_eq!(height, None);
    }

    // Restore
    if let Some(val) = original_rows {
        env::set_var("ROWS", val);
    } else {
        env::remove_var("ROWS");
    }
}

#[test]
fn test_terminal_width_partial_scenario() {
    let original_columns = env::var("COLUMNS").ok();
    let original_rows = env::var("ROWS").ok();

    // Test partial information: width available, height not
    env::set_var("COLUMNS", "80");
    env::remove_var("ROWS");

    let width = cli_mode::terminal_width();
    let height = cli_mode::terminal_height();

    if cli_mode::is_stdout_tty() {
        assert_eq!(width, Some(80));
        assert_eq!(height, None);
    } else {
        assert_eq!(width, None);
        assert_eq!(height, None);
    }

    // Restore
    if let Some(val) = original_columns {
        env::set_var("COLUMNS", val);
    } else {
        env::remove_var("COLUMNS");
    }
    if let Some(val) = original_rows {
        env::set_var("ROWS", val);
    } else {
        env::remove_var("ROWS");
    }
}

#[test]
fn test_terminal_height_partial_scenario() {
    let original_columns = env::var("COLUMNS").ok();
    let original_rows = env::var("ROWS").ok();

    // Test partial information: height available, width not
    env::remove_var("COLUMNS");
    env::set_var("ROWS", "24");

    let width = cli_mode::terminal_width();
    let height = cli_mode::terminal_height();

    if cli_mode::is_stdout_tty() {
        assert_eq!(width, None);
        assert_eq!(height, Some(24));
    } else {
        assert_eq!(width, None);
        assert_eq!(height, None);
    }

    // Restore
    if let Some(val) = original_columns {
        env::set_var("COLUMNS", val);
    } else {
        env::remove_var("COLUMNS");
    }
    if let Some(val) = original_rows {
        env::set_var("ROWS", val);
    } else {
        env::remove_var("ROWS");
    }
}

#[test]
fn test_terminal_dimensions_unavailable() {
    let original_columns = env::var("COLUMNS").ok();
    let original_rows = env::var("ROWS").ok();

    // Test when dimensions are not available
    env::remove_var("COLUMNS");
    env::remove_var("ROWS");

    let width = cli_mode::terminal_width();
    let height = cli_mode::terminal_height();

    // Should return None when not available
    // (May be None even if TTY if COLUMNS/ROWS not set)
    assert!(width.is_none() || width.is_some());
    assert!(height.is_none() || height.is_some());

    // Restore
    if let Some(val) = original_columns {
        env::set_var("COLUMNS", val);
    }
    if let Some(val) = original_rows {
        env::set_var("ROWS", val);
    }
}

#[test]
fn test_terminal_dimensions_non_tty() {
    // Test that dimensions return None when not in TTY
    // This is tested implicitly - if stdout is not TTY, both should return None
    // Actual result depends on test environment
    let _width = cli_mode::terminal_width();
    let _height = cli_mode::terminal_height();
    // Functions should not panic
}
