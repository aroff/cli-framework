//! Tests for output format detection

use cli_framework::cli_mode;
use std::env;
mod common;

#[test]
fn test_output_format_from_env_table() {
    let _guard = common::env_lock().lock().unwrap();
    let original_format = env::var("OUTPUT_FORMAT").ok();

    env::set_var("OUTPUT_FORMAT", "table");
    assert_eq!(
        cli_mode::OutputFormat::from_env(),
        Some(cli_mode::OutputFormat::Table)
    );

    // Test case-insensitive
    env::set_var("OUTPUT_FORMAT", "TABLE");
    assert_eq!(
        cli_mode::OutputFormat::from_env(),
        Some(cli_mode::OutputFormat::Table)
    );

    // Restore
    if let Some(val) = original_format {
        env::set_var("OUTPUT_FORMAT", val);
    } else {
        env::remove_var("OUTPUT_FORMAT");
    }
}

#[test]
fn test_output_format_from_env_json() {
    let _guard = common::env_lock().lock().unwrap();
    let original_format = env::var("OUTPUT_FORMAT").ok();

    env::set_var("OUTPUT_FORMAT", "json");
    assert_eq!(
        cli_mode::OutputFormat::from_env(),
        Some(cli_mode::OutputFormat::Json)
    );

    // Test case-insensitive
    env::set_var("OUTPUT_FORMAT", "JSON");
    assert_eq!(
        cli_mode::OutputFormat::from_env(),
        Some(cli_mode::OutputFormat::Json)
    );

    // Restore
    if let Some(val) = original_format {
        env::set_var("OUTPUT_FORMAT", val);
    } else {
        env::remove_var("OUTPUT_FORMAT");
    }
}

#[test]
fn test_output_format_from_env_plain() {
    let _guard = common::env_lock().lock().unwrap();
    let original_format = env::var("OUTPUT_FORMAT").ok();

    // Ensure clean state
    env::remove_var("OUTPUT_FORMAT");

    env::set_var("OUTPUT_FORMAT", "plain");
    let result = cli_mode::OutputFormat::from_env();
    assert_eq!(
        result,
        Some(cli_mode::OutputFormat::Plain),
        "Expected Plain format, got {:?}",
        result
    );

    // Restore
    if let Some(val) = original_format {
        env::set_var("OUTPUT_FORMAT", val);
    } else {
        env::remove_var("OUTPUT_FORMAT");
    }
}

#[test]
fn test_output_format_from_env_invalid() {
    let _guard = common::env_lock().lock().unwrap();
    let original_format = env::var("OUTPUT_FORMAT").ok();

    env::set_var("OUTPUT_FORMAT", "invalid");
    assert_eq!(cli_mode::OutputFormat::from_env(), None);

    // Restore
    if let Some(val) = original_format {
        env::set_var("OUTPUT_FORMAT", val);
    } else {
        env::remove_var("OUTPUT_FORMAT");
    }
}

#[test]
fn test_output_format_from_env_missing() {
    let _guard = common::env_lock().lock().unwrap();
    let original_format = env::var("OUTPUT_FORMAT").ok();

    // Ensure clean state
    env::remove_var("OUTPUT_FORMAT");
    assert_eq!(cli_mode::OutputFormat::from_env(), None);

    // Restore
    if let Some(val) = original_format {
        env::set_var("OUTPUT_FORMAT", val);
    }
}

#[test]
fn test_output_format_default() {
    // Test that default format is determined by TTY status
    // Result depends on test environment, but function should not panic
    let _format = cli_mode::OutputFormat::default_for_tty();
    // Should return either Table or Json
    assert!(matches!(
        cli_mode::OutputFormat::default_for_tty(),
        cli_mode::OutputFormat::Table | cli_mode::OutputFormat::Json
    ));
}

#[test]
fn test_get_output_format_with_env_var() {
    let _guard = common::env_lock().lock().unwrap();
    let original_format = env::var("OUTPUT_FORMAT").ok();

    // Ensure clean state first
    env::remove_var("OUTPUT_FORMAT");

    // Test that OUTPUT_FORMAT env var takes precedence
    env::set_var("OUTPUT_FORMAT", "plain");
    let result = cli_mode::get_output_format();
    assert_eq!(
        result,
        cli_mode::OutputFormat::Plain,
        "Expected Plain format, got {:?}",
        result
    );

    // Restore
    if let Some(val) = original_format {
        env::set_var("OUTPUT_FORMAT", val);
    } else {
        env::remove_var("OUTPUT_FORMAT");
    }
}

#[test]
fn test_get_output_format_without_env_var() {
    let _guard = common::env_lock().lock().unwrap();
    let original_format = env::var("OUTPUT_FORMAT").ok();

    // Test that default is used when OUTPUT_FORMAT is not set
    env::remove_var("OUTPUT_FORMAT");
    let format = cli_mode::get_output_format();
    // Should return either Table or Json based on TTY
    assert!(matches!(
        format,
        cli_mode::OutputFormat::Table | cli_mode::OutputFormat::Json
    ));

    // Restore
    if let Some(val) = original_format {
        env::set_var("OUTPUT_FORMAT", val);
    }
}

#[test]
fn test_get_output_format_invalid_value_fallback() {
    let _guard = common::env_lock().lock().unwrap();
    let original_format = env::var("OUTPUT_FORMAT").ok();

    // Test that invalid OUTPUT_FORMAT values fallback to default
    // First ensure OUTPUT_FORMAT is not set to a valid value
    env::remove_var("OUTPUT_FORMAT");
    let default_format = cli_mode::get_output_format();

    // Now set to invalid value - should still fallback to default
    env::set_var("OUTPUT_FORMAT", "invalid");
    let format = cli_mode::get_output_format();
    // Should fallback to default (Table or Json based on TTY)
    // The format should match what we got when OUTPUT_FORMAT was unset
    assert_eq!(format, default_format);
    assert!(matches!(
        format,
        cli_mode::OutputFormat::Table | cli_mode::OutputFormat::Json
    ));

    // Restore
    if let Some(val) = original_format {
        env::set_var("OUTPUT_FORMAT", val);
    } else {
        env::remove_var("OUTPUT_FORMAT");
    }
}
