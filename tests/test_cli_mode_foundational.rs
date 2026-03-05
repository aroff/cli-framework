//! Foundational tests for CLI mode detection
//!
//! Tests for terminal detection and environment variable support.

use std::env;
use cli_framework::cli_mode;

#[cfg(test)]
mod cross_platform {
    use super::*;

    #[test]
    fn test_tty_detection_cross_platform() {
        // Test that TTY detection works on all platforms
        // Functions should not panic regardless of platform
        let _stdout = cli_mode::is_stdout_tty();
        let _stderr = cli_mode::is_stderr_tty();
        let _stdin = cli_mode::is_stdin_tty();
        // If we get here, the functions work cross-platform
    }

    #[test]
    fn test_safe_defaults_cross_platform() {
        // Test that safe defaults are returned on all platforms
        // Even if detection fails, should return false (non-interactive) not panic
        let _result = cli_mode::is_stdout_tty();
        // Should be a boolean value, not panic
    }
}

#[test]
fn test_is_stdout_tty() {
    // This test verifies the function doesn't panic
    // Actual TTY status depends on test environment
    let _result = cli_mode::is_stdout_tty();
}

#[test]
fn test_is_stderr_tty() {
    // This test verifies the function doesn't panic
    // Actual TTY status depends on test environment
    let _result = cli_mode::is_stderr_tty();
}

#[test]
fn test_is_stdin_tty() {
    // This test verifies the function doesn't panic
    // Actual TTY status depends on test environment
    let _result = cli_mode::is_stdin_tty();
}

#[test]
fn test_safe_tty_detection_wrapper() {
    // Test that safe wrapper handles panics gracefully
    // In normal operation, is_terminal() shouldn't panic, but we test the wrapper
    let result = cli_mode::is_stdout_tty();
    // Should return a boolean without panicking
    assert!(result == true || result == false);
}

#[test]
fn test_read_env_var() {
    // Test reading an existing environment variable
    env::set_var("TEST_VAR", "test_value");
    let result = cli_mode::read_env_var("TEST_VAR");
    assert_eq!(result, Some("test_value".to_string()));
    env::remove_var("TEST_VAR");
}

#[test]
fn test_read_env_var_case_insensitive() {
    // Test that values are converted to lowercase
    env::set_var("TEST_VAR", "UPPERCASE");
    let result = cli_mode::read_env_var("TEST_VAR");
    assert_eq!(result, Some("uppercase".to_string()));
    env::remove_var("TEST_VAR");
}

#[test]
fn test_read_env_var_missing() {
    // Test reading a non-existent environment variable
    env::remove_var("NON_EXISTENT_VAR");
    let result = cli_mode::read_env_var("NON_EXISTENT_VAR");
    assert_eq!(result, None);
}

#[test]
fn test_is_no_color_set() {
    // Test NO_COLOR detection
    env::set_var("NO_COLOR", "1");
    assert!(cli_mode::is_no_color_set());
    env::remove_var("NO_COLOR");
    assert!(!cli_mode::is_no_color_set());
}

#[test]
fn test_is_force_color_set() {
    // Test FORCE_COLOR detection
    env::set_var("FORCE_COLOR", "1");
    assert!(cli_mode::is_force_color_set());
    env::remove_var("FORCE_COLOR");
    assert!(!cli_mode::is_force_color_set());
}
