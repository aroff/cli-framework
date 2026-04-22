//! Integration tests for AppBuilder

use cli_framework::app::{App, AppBuilder, AppContext};
use cli_framework::command::CommandArgs;

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

#[test]
fn t9_version_string_with_version_configured() {
    let app = AppBuilder::new()
        .with_version("myapp", "1.2.3")
        .build(DummyCtx)
        .unwrap();
    assert_eq!(app.version_string(), "myapp 1.2.3");
}

#[test]
fn t9_version_string_without_version_defaults_to_unknown() {
    let app = AppBuilder::new().build(DummyCtx).unwrap();
    assert_eq!(app.version_string(), "unknown unknown");
}

#[test]
fn t9_execute_command_version_returns_not_found() {
    let mut app = AppBuilder::new()
        .with_version("myapp", "1.2.3")
        .build(DummyCtx)
        .unwrap();

    let rt = tokio::runtime::Runtime::new().unwrap();
    let result = rt.block_on(app.execute_command("version", CommandArgs::default()));
    assert!(result.is_err());
    assert!(result.unwrap_err().to_string().contains("not found"));
}

#[test]
fn t9_show_help_contains_version_entry() {
    let app = AppBuilder::new()
        .with_version("myapp", "1.2.3")
        .build(DummyCtx)
        .unwrap();
    let help = app.render_help();
    assert!(help.contains("version"));
    let version_line = help.lines().next().unwrap();
    assert_eq!(version_line, "  version - Print version information");
}

#[test]
fn t9_show_help_version_appears_before_registered_commands() {
    use cli_framework::command::Command;

    let cmd = Command {
        id: "alpha",
        summary: "Alpha command",
        syntax: None,
        category: Some("test"),
        execute: |_ctx, _args| Box::pin(async move { Ok(()) }),
    };

    let app = AppBuilder::new()
        .with_version("myapp", "1.2.3")
        .register_command(cmd)
        .build(DummyCtx)
        .unwrap();
    let help = app.render_help();
    let version_pos = help.find("version - Print version information").unwrap();
    let alpha_pos = help.find("alpha").unwrap();
    assert!(version_pos < alpha_pos);
}

#[test]
fn t9_version_with_custom_name_and_version() {
    let app = AppBuilder::new()
        .with_version("fastskill", "0.9.101")
        .build(DummyCtx)
        .unwrap();
    assert_eq!(app.version_string(), "fastskill 0.9.101");
}

#[test]
fn t9_version_builder_chain_order_does_not_matter() {
    use cli_framework::command::Command;

    let cmd = Command {
        id: "test",
        summary: "Test command",
        syntax: None,
        category: None,
        execute: |_ctx, _args| Box::pin(async move { Ok(()) }),
    };

    let app = AppBuilder::new()
        .register_command(cmd)
        .with_version("chain-test", "2.0.0")
        .build(DummyCtx)
        .unwrap();
    assert_eq!(app.version_string(), "chain-test 2.0.0");
}
