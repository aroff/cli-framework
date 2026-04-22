//! Integration tests for AppBuilder

use cli_framework::app::{App, AppContext};

struct DummyCtx;
impl AppContext for DummyCtx {}

#[test]
fn t9_app_should_show_help_predicate_matches_expected_inputs() {
    let args = vec!["prog".to_string(), "--help".to_string()];
    assert!(App::<DummyCtx>::should_show_help(&args));

    let args = vec!["prog".to_string(), "-h".to_string()];
    assert!(App::<DummyCtx>::should_show_help(&args));

    let args = vec!["prog".to_string()];
    assert!(App::<DummyCtx>::should_show_help(&args));

    let args = vec!["prog".to_string(), "status".to_string()];
    assert!(!App::<DummyCtx>::should_show_help(&args));
}
