//! Unit tests for `mcp install` and `mcp list` auto-registration (feature: mcp-install).

use cli_framework::app::{AppBuilder, AppContext};
use cli_framework::spec::command_tree::CommandPath;

struct DummyCtx;
impl AppContext for DummyCtx {}

/// mcp/install is registered after build() when mcp-install is enabled.
#[test]
fn mcp_install_registered_after_build() {
    let app = AppBuilder::new()
        .with_version("testapp", "0.1.0")
        .build(DummyCtx)
        .unwrap();

    let path = CommandPath::new(&["mcp", "install"]).unwrap();
    let found = app.command_registry().resolve(&path).is_some();
    assert!(found, "mcp/install not registered after build()");
}

/// mcp/register alias is registered after build().
#[test]
fn mcp_register_alias_registered_after_build() {
    let app = AppBuilder::new()
        .with_version("testapp", "0.1.0")
        .build(DummyCtx)
        .unwrap();

    let path = CommandPath::new(&["mcp", "register"]).unwrap();
    let found = app.command_registry().resolve(&path).is_some();
    assert!(found, "mcp/register not registered after build()");
}

/// mcp/list is registered after build() when mcp-install is enabled.
#[test]
fn mcp_list_registered_after_build() {
    let app = AppBuilder::new()
        .with_version("testapp", "0.1.0")
        .build(DummyCtx)
        .unwrap();

    let path = CommandPath::new(&["mcp", "list"]).unwrap();
    let found = app.command_registry().resolve(&path).is_some();
    assert!(found, "mcp/list not registered after build()");
}

/// `mcp install --dry-run` prints what would be done and returns Ok.
#[tokio::test]
async fn mcp_install_dry_run_succeeds() {
    let mut app = AppBuilder::new()
        .with_version("testapp", "0.1.0")
        .build(DummyCtx)
        .unwrap();

    let result = app
        .run_with_args(vec![
            "testapp".to_string(),
            "mcp".to_string(),
            "install".to_string(),
            "--dry-run".to_string(),
        ])
        .await;

    assert!(result.is_ok(), "mcp install --dry-run failed: {:?}", result);
}

/// `mcp list` prints the agent table and returns Ok.
#[tokio::test]
async fn mcp_list_prints_agents() {
    let mut app = AppBuilder::new()
        .with_version("testapp", "0.1.0")
        .build(DummyCtx)
        .unwrap();

    let result = app
        .run_with_args(vec![
            "testapp".to_string(),
            "mcp".to_string(),
            "list".to_string(),
        ])
        .await;

    assert!(result.is_ok(), "mcp list failed: {:?}", result);
}

/// `mcp install` with an unknown agent key triggers `McpDeployError` → E011.
#[tokio::test]
async fn mcp_install_unknown_agent_returns_e011() {
    let mut app = AppBuilder::new()
        .with_version("testapp", "0.1.0")
        .build(DummyCtx)
        .unwrap();

    let result = app
        .run_with_args(vec![
            "testapp".to_string(),
            "mcp".to_string(),
            "install".to_string(),
            "--agent".to_string(),
            "not-a-real-agent".to_string(),
        ])
        .await;

    assert!(result.is_err(), "expected error for unknown agent");
    let err_msg = result.unwrap_err().to_string();
    assert!(
        err_msg.contains("E011"),
        "expected E011 error code in: {}",
        err_msg
    );
}

/// `mcp install --dry-run --stdio` prints stdio dry-run message and returns Ok.
#[tokio::test]
async fn mcp_install_dry_run_stdio_succeeds() {
    let mut app = AppBuilder::new()
        .with_version("testapp", "0.1.0")
        .build(DummyCtx)
        .unwrap();

    let result = app
        .run_with_args(vec![
            "testapp".to_string(),
            "mcp".to_string(),
            "install".to_string(),
            "--stdio".to_string(),
            "--dry-run".to_string(),
        ])
        .await;

    assert!(
        result.is_ok(),
        "mcp install --stdio --dry-run failed: {:?}",
        result
    );
}

/// `mcp install --stdio` defaults argv to `mcp serve --transport stdio` when no `--arg` provided.
#[tokio::test]
async fn mcp_install_stdio_defaults_argv() {
    let tempdir = tempfile::tempdir().expect("tempdir");

    let mut app = AppBuilder::new()
        .with_version("testapp", "0.1.0")
        .build(DummyCtx)
        .unwrap();

    let result = app
        .run_with_args(vec![
            "testapp".to_string(),
            "mcp".to_string(),
            "install".to_string(),
            "--stdio".to_string(),
            "--scope".to_string(),
            "project".to_string(),
            "--project".to_string(),
            tempdir.path().to_str().unwrap().to_string(),
            "--agent".to_string(),
            "claude".to_string(),
            "--overwrite".to_string(),
        ])
        .await;

    assert!(result.is_ok(), "mcp install --stdio failed: {:?}", result);

    let path = aikit_sdk::mcp_config_path("claude", aikit_sdk::McpScope::Project, tempdir.path())
        .expect("mcp_config_path");

    let contents = std::fs::read_to_string(&path).expect("read config");
    let json: serde_json::Value = serde_json::from_str(&contents).expect("parse config json");

    let args = json
        .pointer("/mcpServers/testapp/args")
        .and_then(|v| v.as_array())
        .expect("expected mcpServers.testapp.args array");

    let args: Vec<String> = args
        .iter()
        .filter_map(|v| v.as_str().map(|s| s.to_string()))
        .collect();

    assert_eq!(
        args,
        vec!["mcp", "serve", "--transport", "stdio"],
        "unexpected default argv: {:?}",
        args
    );
}
