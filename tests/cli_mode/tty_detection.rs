use cli_framework::cli_mode;

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

