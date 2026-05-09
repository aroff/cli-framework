//! Integration tests for multi-segment CommandPath routing (AC6, AC14, §4.3).

use cli_framework::app::{AppBuilder, AppContext};
use cli_framework::command::{Command, CommandArgs};
use cli_framework::spec::command_tree::{CommandPath, CommandSpec, GroupMetadata};
use std::sync::{Arc, Mutex};

struct DummyCtx;
impl AppContext for DummyCtx {}

fn make_tracking_cmd(id: &'static str, executed: Arc<Mutex<bool>>) -> Command {
    Command {
        id,
        summary: "Test command",
        syntax: None,
        category: None,
        spec: Some(Arc::new(CommandSpec {
            summary: "Test command",
            ..Default::default()
        })),
        validator: None,
        expose_mcp: false,
        execute: Arc::new(move |_ctx, _args: CommandArgs| {
            let executed = Arc::clone(&executed);
            Box::pin(async move {
                *executed.lock().unwrap() = true;
                Ok(())
            })
        }),
    }
}

/// AC6: A command registered at ["cluster", "get"] is reachable via `prog cluster get`.
#[tokio::test]
async fn nested_command_executes_via_cli() {
    let executed = Arc::new(Mutex::new(false));
    let get_cmd = make_tracking_cmd("get", Arc::clone(&executed));

    let get_path = CommandPath::new(&["cluster", "get"]).unwrap();
    let group_path = CommandPath::root_for("cluster");

    let mut app = AppBuilder::new()
        .with_version("testapp", "0.1.0")
        .register_group(
            &group_path,
            GroupMetadata {
                summary: "Cluster management",
                hidden: false,
            },
        )
        .unwrap()
        .register_command_at(&get_path, get_cmd)
        .unwrap()
        .build(DummyCtx)
        .unwrap();

    let result = app
        .run_with_args(vec![
            "testapp".to_string(),
            "cluster".to_string(),
            "get".to_string(),
        ])
        .await;

    assert!(result.is_ok(), "run_with_args returned error: {:?}", result);
    assert!(
        *executed.lock().unwrap(),
        "cluster get execute closure was not called"
    );
}

/// AC14: `registry.resolve(&["mcp", "serve"])` returns `Some(_)` after `build()`.
#[cfg(feature = "mcp-server")]
#[test]
fn mcp_serve_registered_after_build() {
    let app = AppBuilder::new()
        .with_version("testapp", "0.1.0")
        .build(DummyCtx)
        .unwrap();

    let path = CommandPath::new(&["mcp", "serve"]).unwrap();
    let found = app.command_registry().resolve(&path).is_some();
    assert!(found, "mcp/serve not registered after build()");
}

/// AC13: `prog unknown-group sub` returns E012 at the parse layer.
#[test]
fn unknown_nested_command_parse_error_e012() {
    use cli_framework::app::clap_adapter::{build_clap_root, parse_with_clap};
    use cli_framework::command::CommandRegistry;
    use cli_framework::parser::{error_codes::E_NESTED_COMMAND_NOT_FOUND, outcome::ParseOutcome};

    let registry = CommandRegistry::new();
    let root = build_clap_root(None, &registry, "testapp", "0.1.0");

    let outcome = parse_with_clap(
        &root,
        &registry,
        vec![
            "testapp".to_string(),
            "unknown-group".to_string(),
            "sub".to_string(),
        ],
    );

    match outcome {
        ParseOutcome::ParseError(d) => {
            assert_eq!(
                d.code, E_NESTED_COMMAND_NOT_FOUND,
                "expected E012, got: {}",
                d.code
            );
        }
        other => panic!("expected ParseError(E012), got {:?}", other),
    }
}
