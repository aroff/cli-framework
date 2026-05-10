//! Tests for color output detection

use cli_framework::cli_mode;
use std::env;

mod common;

#[test]
fn test_color_detection_no_color_set() {
    let _guard = common::env_lock().lock().unwrap();
    // Test that NO_COLOR disables colors regardless of TTY status
    let original_no_color = env::var("NO_COLOR").ok();
    let original_force_color = env::var("FORCE_COLOR").ok();

    env::set_var("NO_COLOR", "1");
    env::remove_var("FORCE_COLOR");

    assert!(
        !cli_mode::should_color_output(),
        "Colors should be disabled when NO_COLOR is set"
    );
    assert!(
        !cli_mode::should_color_stderr(),
        "Stderr colors should be disabled when NO_COLOR is set"
    );

    // Restore original values
    if let Some(val) = original_no_color {
        env::set_var("NO_COLOR", val);
    } else {
        env::remove_var("NO_COLOR");
    }
    if let Some(val) = original_force_color {
        env::set_var("FORCE_COLOR", val);
    } else {
        env::remove_var("FORCE_COLOR");
    }
}

#[test]
fn test_color_detection_force_color_set() {
    let _guard = common::env_lock().lock().unwrap();
    // Test that FORCE_COLOR enables colors when NO_COLOR is not set
    let original_no_color = env::var("NO_COLOR").ok();
    let original_force_color = env::var("FORCE_COLOR").ok();

    env::remove_var("NO_COLOR");
    env::set_var("FORCE_COLOR", "1");

    assert!(
        cli_mode::should_color_output(),
        "Colors should be enabled when FORCE_COLOR is set and NO_COLOR is not"
    );
    assert!(
        cli_mode::should_color_stderr(),
        "Stderr colors should be enabled when FORCE_COLOR is set and NO_COLOR is not"
    );

    // Restore original values
    if let Some(val) = original_no_color {
        env::set_var("NO_COLOR", val);
    } else {
        env::remove_var("NO_COLOR");
    }
    if let Some(val) = original_force_color {
        env::set_var("FORCE_COLOR", val);
    } else {
        env::remove_var("FORCE_COLOR");
    }
}

#[test]
fn test_color_detection_no_env_vars() {
    let _guard = common::env_lock().lock().unwrap();
    // Test TTY-based detection when no environment variables are set
    let original_no_color = env::var("NO_COLOR").ok();
    let original_force_color = env::var("FORCE_COLOR").ok();

    env::remove_var("NO_COLOR");
    env::remove_var("FORCE_COLOR");

    // Result depends on TTY status, but function should not panic
    let _stdout_result = cli_mode::should_color_output();
    let _stderr_result = cli_mode::should_color_stderr();

    // Restore original values
    if let Some(val) = original_no_color {
        env::set_var("NO_COLOR", val);
    } else {
        env::remove_var("NO_COLOR");
    }
    if let Some(val) = original_force_color {
        env::set_var("FORCE_COLOR", val);
    } else {
        env::remove_var("FORCE_COLOR");
    }
}

#[test]
fn test_color_detection_no_color_overrides_force_color() {
    let _guard = common::env_lock().lock().unwrap();
    // Test that NO_COLOR takes precedence over FORCE_COLOR
    let original_no_color = env::var("NO_COLOR").ok();
    let original_force_color = env::var("FORCE_COLOR").ok();

    env::set_var("NO_COLOR", "1");
    env::set_var("FORCE_COLOR", "1");

    assert!(
        !cli_mode::should_color_output(),
        "NO_COLOR should override FORCE_COLOR"
    );
    assert!(
        !cli_mode::should_color_stderr(),
        "NO_COLOR should override FORCE_COLOR for stderr"
    );

    // Restore original values
    if let Some(val) = original_no_color {
        env::set_var("NO_COLOR", val);
    } else {
        env::remove_var("NO_COLOR");
    }
    if let Some(val) = original_force_color {
        env::set_var("FORCE_COLOR", val);
    } else {
        env::remove_var("FORCE_COLOR");
    }
}

#[test]
fn test_color_detection_detection_failure() {
    let _guard = common::env_lock().lock().unwrap();
    // Test that detection failures return safe default (false)
    // This is tested implicitly through the safe_tty_check wrapper
    // The function should never panic, even if TTY detection fails
    let _result = cli_mode::should_color_output();
    let _result2 = cli_mode::should_color_stderr();
    // If we get here without panicking, the test passes
}
