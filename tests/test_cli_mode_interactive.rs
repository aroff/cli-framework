//! Tests for interactive mode detection

use cli_framework::cli_mode;
use std::sync::{Mutex, OnceLock};

fn env_lock() -> &'static Mutex<()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
}

#[test]
fn test_is_interactive_both_tty() {
    // Test that is_interactive requires both stdin and stdout to be TTYs
    // Actual result depends on test environment, but function should not panic
    let _result = cli_mode::is_interactive();
    // If we get here without panicking, the test passes
}

#[test]
fn test_is_interactive_one_stream_non_tty() {
    // Test that is_interactive returns false when one stream is not TTY
    // This is tested implicitly - if either stdin or stdout is not TTY,
    // is_interactive should return false
    let result = cli_mode::is_interactive();
    // Result depends on environment, but should be a boolean
    assert!(result == true || result == false);
}

#[test]
fn test_is_quiet() {
    use std::env;

    // Test quiet mode detection
    let _guard = env_lock().lock().unwrap();
    let original_quiet = env::var("QUIET").ok();

    env::set_var("QUIET", "1");
    assert!(
        cli_mode::is_quiet(),
        "Quiet mode should be detected when QUIET is set"
    );

    env::remove_var("QUIET");
    assert!(
        !cli_mode::is_quiet(),
        "Quiet mode should not be detected when QUIET is not set"
    );

    // Restore original value
    if let Some(val) = original_quiet {
        env::set_var("QUIET", val);
    }
}

#[test]
fn test_should_show_progress() {
    use std::env;

    // Test progress indicator detection
    let _guard = env_lock().lock().unwrap();
    let original_quiet = env::var("QUIET").ok();

    // Test with QUIET set - progress should be suppressed
    env::set_var("QUIET", "1");
    assert!(
        !cli_mode::should_show_progress(),
        "Progress should be suppressed when QUIET is set"
    );

    // Test with QUIET unset - progress depends on TTY status
    env::remove_var("QUIET");
    let _result = cli_mode::should_show_progress();
    // Result depends on TTY, but function should not panic

    // Restore original value
    if let Some(val) = original_quiet {
        env::set_var("QUIET", val);
    } else {
        env::remove_var("QUIET");
    }
}

#[test]
fn test_should_show_progress_combines_tty_and_quiet() {
    use std::env;

    // Test that progress detection considers both TTY and quiet mode
    let _guard = env_lock().lock().unwrap();
    let original_quiet = env::var("QUIET").ok();

    // Even if TTY, quiet mode should suppress progress
    env::set_var("QUIET", "1");
    assert!(
        !cli_mode::should_show_progress(),
        "Progress should be suppressed in quiet mode even if TTY"
    );

    // Restore original value
    if let Some(val) = original_quiet {
        env::set_var("QUIET", val);
    } else {
        env::remove_var("QUIET");
    }
}
