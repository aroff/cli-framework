use std::env;
use cli_framework::cli_mode;

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

