//! Tests for terminal dimension detection.
//!
//! `COLUMNS` and `ROWS` are honored unconditionally (TTY not required).
//! The syscall fallback remains guarded by TTY.

use cli_framework::cli_mode;
use std::env;
use std::sync::Mutex;

// Serialize all env-var tests to avoid race conditions between parallel test threads.
static ENV_LOCK: Mutex<()> = Mutex::new(());

#[test]
fn test_terminal_width_from_env() {
    let _guard = ENV_LOCK.lock().unwrap();
    let original = env::var("COLUMNS").ok();

    env::set_var("COLUMNS", "120");
    let width = cli_mode::terminal_width();
    // COLUMNS is read unconditionally, regardless of TTY.
    assert_eq!(width, Some(120));

    match original {
        Some(val) => env::set_var("COLUMNS", val),
        None => env::remove_var("COLUMNS"),
    }
}

#[test]
fn test_terminal_height_from_env() {
    let _guard = ENV_LOCK.lock().unwrap();
    let original = env::var("ROWS").ok();

    env::set_var("ROWS", "40");
    let height = cli_mode::terminal_height();
    // ROWS is read unconditionally, regardless of TTY.
    assert_eq!(height, Some(40));

    match original {
        Some(val) => env::set_var("ROWS", val),
        None => env::remove_var("ROWS"),
    }
}

#[test]
fn test_terminal_width_partial_scenario() {
    let _guard = ENV_LOCK.lock().unwrap();
    let orig_col = env::var("COLUMNS").ok();
    let orig_row = env::var("ROWS").ok();

    env::set_var("COLUMNS", "80");
    env::remove_var("ROWS");

    // COLUMNS set → Some(80); ROWS not set + non-TTY → None.
    assert_eq!(cli_mode::terminal_width(), Some(80));
    if !cli_mode::is_stdout_tty() {
        assert_eq!(cli_mode::terminal_height(), None);
    }

    match orig_col {
        Some(v) => env::set_var("COLUMNS", v),
        None => env::remove_var("COLUMNS"),
    }
    match orig_row {
        Some(v) => env::set_var("ROWS", v),
        None => env::remove_var("ROWS"),
    }
}

#[test]
fn test_terminal_height_partial_scenario() {
    let _guard = ENV_LOCK.lock().unwrap();
    let orig_col = env::var("COLUMNS").ok();
    let orig_row = env::var("ROWS").ok();

    env::remove_var("COLUMNS");
    env::set_var("ROWS", "24");

    // ROWS set → Some(24); COLUMNS not set + non-TTY → None.
    assert_eq!(cli_mode::terminal_height(), Some(24));
    if !cli_mode::is_stdout_tty() {
        assert_eq!(cli_mode::terminal_width(), None);
    }

    match orig_col {
        Some(v) => env::set_var("COLUMNS", v),
        None => env::remove_var("COLUMNS"),
    }
    match orig_row {
        Some(v) => env::set_var("ROWS", v),
        None => env::remove_var("ROWS"),
    }
}

#[test]
fn test_terminal_dimensions_unavailable() {
    let _guard = ENV_LOCK.lock().unwrap();
    let orig_col = env::var("COLUMNS").ok();
    let orig_row = env::var("ROWS").ok();

    env::remove_var("COLUMNS");
    env::remove_var("ROWS");

    // No env vars set; syscall fallback requires TTY.
    if !cli_mode::is_stdout_tty() {
        assert_eq!(cli_mode::terminal_width(), None);
        assert_eq!(cli_mode::terminal_height(), None);
    }

    match orig_col {
        Some(v) => env::set_var("COLUMNS", v),
        None => {}
    }
    match orig_row {
        Some(v) => env::set_var("ROWS", v),
        None => {}
    }
}

#[test]
fn test_terminal_dimensions_non_tty() {
    // Functions must not panic regardless of TTY state.
    let _width = cli_mode::terminal_width();
    let _height = cli_mode::terminal_height();
}
