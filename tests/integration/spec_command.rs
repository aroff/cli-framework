//! Integration tests for the built-in `spec` command.

use cli_framework::app::{AppBuilder, AppContext};
use cli_framework::command::Command;
use cli_framework::spec::command_tree::CommandSpec;
use std::sync::Arc;

struct DummyCtx;
impl AppContext for DummyCtx {}

async fn run_spec_to_tempfile<C: AppContext>(
    app: &mut cli_framework::app::App<C>,
) -> anyhow::Result<String> {
    let tmp = tempfile::NamedTempFile::new()?;
    let path = tmp.path().to_string_lossy().to_string();

    app.run_with_args(vec![
        "myapp".to_string(),
        "spec".to_string(),
        "--output".to_string(),
        path.clone(),
    ])
    .await?;

    Ok(std::fs::read_to_string(path)?)
}

fn noop_command(id: &'static str) -> Command {
    Command {
        id: Arc::from(id),
        spec: Arc::new(CommandSpec {
            summary: "Test command",
            ..Default::default()
        }),
        validator: None,
        expose_mcp: false,
        expose_chat: true,
        meta: None,
        visibility: None,
        execute: Arc::new(|_ctx, _args| Box::pin(async { Ok(()) })),
    }
}

fn hidden_command() -> Command {
    Command {
        id: Arc::from("internal"),
        spec: Arc::new(CommandSpec {
            summary: "Hidden internal command",
            hidden: true,
            ..Default::default()
        }),
        validator: None,
        expose_mcp: false,
        expose_chat: true,
        meta: None,
        visibility: None,
        execute: Arc::new(|_ctx, _args| Box::pin(async { Ok(()) })),
    }
}

// ── JSON stdout output ────────────────────────────────────────────────────────

#[tokio::test]
async fn spec_json_stdout_is_valid() {
    let mut app = AppBuilder::new()
        .register_command(noop_command("deploy"))
        .unwrap()
        .build(DummyCtx)
        .unwrap();

    let output = run_spec_to_tempfile(&mut app)
        .await
        .expect("spec command should succeed");

    let parsed: serde_json::Value =
        serde_json::from_str(&output).expect("spec output should be valid JSON");

    assert_eq!(
        parsed["schemaVersion"].as_str(),
        Some("cli-framework.command-surface.v1"),
        "schemaVersion must match"
    );
    assert!(parsed["commands"].is_array(), "commands must be array");

    let commands = parsed["commands"].as_array().unwrap();
    let has_deploy = commands
        .iter()
        .any(|c| c["path"].as_str() == Some("deploy"));
    assert!(has_deploy, "deploy command should appear in spec output");
}

// ── YAML stdout output ────────────────────────────────────────────────────────

#[tokio::test]
async fn spec_yaml_stdout_is_valid() {
    let mut app = AppBuilder::new()
        .register_command(noop_command("deploy"))
        .unwrap()
        .build(DummyCtx)
        .unwrap();

    let tmp = tempfile::NamedTempFile::new().unwrap();
    let path = tmp.path().to_string_lossy().to_string();

    let result = app
        .run_with_args(vec![
            "myapp".to_string(),
            "spec".to_string(),
            "--format".to_string(),
            "yaml".to_string(),
            "--output".to_string(),
            path.clone(),
        ])
        .await;

    assert!(
        result.is_ok(),
        "spec --format yaml should succeed: {:?}",
        result
    );
    let output = std::fs::read_to_string(path).unwrap_or_default();
    assert!(!output.is_empty(), "YAML output should not be empty");
    assert!(
        output.contains("schemaVersion") || output.contains("schema_version"),
        "YAML output should contain schema version key"
    );
}

// ── Markdown stdout output ────────────────────────────────────────────────────

#[tokio::test]
async fn spec_markdown_stdout_has_headings() {
    let mut app = AppBuilder::new()
        .register_command(noop_command("deploy"))
        .unwrap()
        .build(DummyCtx)
        .unwrap();

    let tmp = tempfile::NamedTempFile::new().unwrap();
    let path = tmp.path().to_string_lossy().to_string();

    let result = app
        .run_with_args(vec![
            "myapp".to_string(),
            "spec".to_string(),
            "--format".to_string(),
            "markdown".to_string(),
            "--output".to_string(),
            path.clone(),
        ])
        .await;

    assert!(
        result.is_ok(),
        "spec --format markdown should succeed: {:?}",
        result
    );
    let output = std::fs::read_to_string(path).unwrap_or_default();
    assert!(
        output.contains("## deploy"),
        "Markdown should have ## deploy heading"
    );
}

// ── --output writes to file ───────────────────────────────────────────────────

#[tokio::test]
async fn spec_output_writes_to_file() {
    let tmp = tempfile::NamedTempFile::new().unwrap();
    let path = tmp.path().to_string_lossy().to_string();

    let mut app = AppBuilder::new()
        .register_command(noop_command("deploy"))
        .unwrap()
        .build(DummyCtx)
        .unwrap();

    let result = app
        .run_with_args(vec![
            "myapp".to_string(),
            "spec".to_string(),
            "--output".to_string(),
            path.clone(),
        ])
        .await;

    assert!(result.is_ok(), "spec --output should succeed: {:?}", result);

    let file_content = std::fs::read_to_string(&path).expect("output file should exist");
    let parsed: serde_json::Value =
        serde_json::from_str(&file_content).expect("file content should be valid JSON");
    assert_eq!(
        parsed["schemaVersion"].as_str(),
        Some("cli-framework.command-surface.v1")
    );
}

// ── --include-hidden excludes hidden without flag ─────────────────────────────

#[tokio::test]
async fn spec_hidden_command_excluded() {
    let mut app = AppBuilder::new()
        .register_command(noop_command("visible"))
        .unwrap()
        .register_command(hidden_command())
        .unwrap()
        .build(DummyCtx)
        .unwrap();

    let output = run_spec_to_tempfile(&mut app).await.unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&output).unwrap();
    let commands = parsed["commands"].as_array().unwrap();
    let has_internal = commands
        .iter()
        .any(|c| c["path"].as_str() == Some("internal"));
    assert!(
        !has_internal,
        "hidden command should not appear without --include-hidden"
    );
}

// ── --include-hidden includes hidden with flag ────────────────────────────────

#[tokio::test]
async fn spec_include_hidden_flag_includes_hidden() {
    let mut app = AppBuilder::new()
        .register_command(noop_command("visible"))
        .unwrap()
        .register_command(hidden_command())
        .unwrap()
        .build(DummyCtx)
        .unwrap();

    let tmp = tempfile::NamedTempFile::new().unwrap();
    let path = tmp.path().to_string_lossy().to_string();
    let result = app
        .run_with_args(vec![
            "myapp".to_string(),
            "spec".to_string(),
            "--include-hidden".to_string(),
            "--output".to_string(),
            path.clone(),
        ])
        .await;

    assert!(result.is_ok());
    let output = std::fs::read_to_string(path).unwrap_or_default();
    let parsed: serde_json::Value = serde_json::from_str(&output).unwrap();
    let commands = parsed["commands"].as_array().unwrap();
    let has_internal = commands
        .iter()
        .any(|c| c["path"].as_str() == Some("internal"));
    assert!(
        has_internal,
        "hidden command should appear with --include-hidden"
    );
}

// ── CS001 / R4a: unknown format rejected as usage error ─────────────────────
//
// With R4a Enum validation, `--format html` is rejected at parse time (E004)
// before the command handler runs.  The error is a UsageError (exit 2), and
// the message names the invalid value and the allowed set.

#[tokio::test]
async fn spec_format_html_rejected_as_usage_error() {
    let mut app = AppBuilder::new()
        .register_command(noop_command("deploy"))
        .unwrap()
        .build(DummyCtx)
        .unwrap();

    let result = app
        .run_with_args(vec![
            "myapp".to_string(),
            "spec".to_string(),
            "--format".to_string(),
            "html".to_string(),
        ])
        .await;

    assert!(result.is_err(), "spec --format html should fail");
    let err = result.unwrap_err();
    // R4a: parse-time Enum validation returns UsageError (exit 2).
    assert!(
        err.downcast_ref::<cli_framework::UsageError>().is_some(),
        "expected UsageError for invalid Enum value, got: {}",
        err
    );
    let msg = err.to_string();
    assert!(
        msg.contains("html"),
        "error should name the invalid value, got: {}",
        msg
    );
}

// ── CS002: invalid output path ────────────────────────────────────────────────

#[tokio::test]
async fn spec_output_invalid_path_cs002_error() {
    let mut app = AppBuilder::new()
        .register_command(noop_command("deploy"))
        .unwrap()
        .build(DummyCtx)
        .unwrap();

    let result = app
        .run_with_args(vec![
            "myapp".to_string(),
            "spec".to_string(),
            "--output".to_string(),
            "/nonexistent_dir_for_test/out.json".to_string(),
        ])
        .await;

    assert!(result.is_err(), "spec with invalid output path should fail");
    let err_msg = result.unwrap_err().to_string();
    assert!(
        err_msg.contains("CS002"),
        "error should contain CS002, got: {}",
        err_msg
    );
}

// ── Collision: user-registered 'spec' not overwritten ────────────────────────

#[test]
fn user_spec_command_not_overwritten() {
    let custom_spec = Command {
        id: Arc::from("spec"),
        spec: Arc::new(CommandSpec {
            summary: "Custom spec command",
            ..Default::default()
        }),
        validator: None,
        expose_mcp: false,
        expose_chat: true,
        meta: None,
        visibility: None,
        execute: Arc::new(|_ctx, _args| Box::pin(async { Ok(()) })),
    };

    // Should succeed: collision guard prevents override, no panic
    let result = AppBuilder::new()
        .register_command(custom_spec)
        .unwrap()
        .build(DummyCtx);

    assert!(
        result.is_ok(),
        "build should succeed when user registers their own spec command"
    );
}
