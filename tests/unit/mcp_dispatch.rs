use cli_framework::app::{AppBuilder, AppContext};
use cli_framework::command::{Command, CommandRegistry};
use cli_framework::mcp::{
    dispatch_tool_call, dispatch_tool_call_spawned, McpToolRegistry, McpTransportKind,
};
use cli_framework::security::command_risk::CommandRiskTier;
use cli_framework::security::gate::{ExecutionGate, GateError};
use cli_framework::spec::arg_spec::{ArgKind, ArgSpec, ArgValueType, Cardinality};
use cli_framework::spec::command_tree::{CommandPath, CommandSpec};
use cli_framework::spec::value::ArgValue;
use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

struct DummyCtx;
impl AppContext for DummyCtx {}

/// Stage 2 requirement: `mcp serve` is auto-registered after `build()` when `mcp-server` is on.
#[cfg(feature = "mcp-server")]
#[test]
fn mcp_serve_registered_after_build() {
    let app = AppBuilder::new()
        .with_version("testapp", "0.1.0")
        .build(DummyCtx)
        .unwrap();

    let path = CommandPath::new(&["mcp", "serve"]).unwrap();
    let found = app.command_registry().resolve(&path).is_some();
    assert!(found, "mcp/serve not registered in registry after build()");
}

fn noop_execute() -> Arc<
    dyn for<'a> Fn(
            &'a mut dyn cli_framework::app::AppContext,
            HashMap<String, ArgValue>,
        ) -> Pin<Box<dyn Future<Output = anyhow::Result<()>> + Send + 'a>>
        + Send
        + Sync,
> {
    Arc::new(|_ctx, _args| Box::pin(async { Ok(()) }))
}

fn failing_execute() -> Arc<
    dyn for<'a> Fn(
            &'a mut dyn cli_framework::app::AppContext,
            HashMap<String, ArgValue>,
        ) -> Pin<Box<dyn Future<Output = anyhow::Result<()>> + Send + 'a>>
        + Send
        + Sync,
> {
    Arc::new(|_ctx, _args| Box::pin(async { Err(anyhow::anyhow!("command execution failed")) }))
}

fn make_cmd(id: &'static str) -> Command {
    Command {
        id: Arc::from(id),
        spec: Arc::new(CommandSpec {
            summary: "test command",
            ..Default::default()
        }),
        validator: None,
        expose_mcp: false,
        expose_chat: true,
        meta: None,
        visibility: None,
        execute: noop_execute(),
    }
}

fn make_registry_with_cmd(_id: &'static str, cmd: Command) -> McpToolRegistry {
    let mut registry = CommandRegistry::new();
    registry.register(cmd);
    McpToolRegistry::from_command_registry(&registry, "myapp")
}

#[tokio::test]
async fn test_tool_call_success() {
    let cmd = make_cmd("hello");
    let tool_registry = make_registry_with_cmd("hello", cmd);

    let result =
        dispatch_tool_call(&tool_registry, "myapp_hello", None, McpTransportKind::Http).await;
    assert!(result.is_ok());
    let call_result = result.unwrap();
    assert_eq!(call_result.is_error, Some(false));
}

#[tokio::test]
async fn test_tool_call_cmd_not_found() {
    let tool_registry = McpToolRegistry::from_command_registry(&CommandRegistry::new(), "myapp");

    let result = dispatch_tool_call(
        &tool_registry,
        "myapp_nonexistent",
        None,
        McpTransportKind::Http,
    )
    .await;
    assert!(result.is_err());
    let err = result.unwrap_err();
    assert!(
        err.message.starts_with("MCP_CMD_NOT_FOUND:"),
        "got: {}",
        err.message
    );
}

#[tokio::test]
async fn test_tool_call_arg_validation_failed() {
    let spec = CommandSpec {
        args: vec![ArgSpec {
            name: "required-arg",
            kind: ArgKind::Option,
            short: None,
            long: None,
            value_type: ArgValueType::String,
            cardinality: Cardinality::Required,
            default: None,
            conflicts_with: vec![],
            requires: vec![],
            help: "A required argument",
            ..Default::default()
        }],
        ..Default::default()
    };
    let cmd = Command {
        id: Arc::from("test-cmd"),
        spec: Arc::new(spec),
        validator: None,
        expose_mcp: false,
        expose_chat: true,
        meta: None,
        visibility: None,
        execute: noop_execute(),
    };

    let mut registry = CommandRegistry::new();
    registry.register(cmd);
    let tool_registry = McpToolRegistry::from_command_registry(&registry, "myapp");

    // Call without required arg
    let result = dispatch_tool_call(
        &tool_registry,
        "myapp_test-cmd",
        None,
        McpTransportKind::Http,
    )
    .await;
    assert!(result.is_err());
    let err = result.unwrap_err();
    assert!(
        err.message.starts_with("MCP_ARG_VALIDATION_FAILED:"),
        "got: {}",
        err.message
    );
}

#[tokio::test]
async fn test_tool_call_execution_failed() {
    let cmd = Command {
        id: Arc::from("fail-cmd"),
        spec: Arc::new(CommandSpec {
            summary: "failing command",
            ..Default::default()
        }),
        validator: None,
        expose_mcp: false,
        expose_chat: true,
        meta: None,
        visibility: None,
        execute: failing_execute(),
    };

    let tool_registry = make_registry_with_cmd("fail-cmd", cmd);

    let result = dispatch_tool_call(
        &tool_registry,
        "myapp_fail-cmd",
        None,
        McpTransportKind::Http,
    )
    .await;
    assert!(result.is_err());
    let err = result.unwrap_err();
    assert!(
        err.message.starts_with("MCP_EXECUTION_FAILED:"),
        "got: {}",
        err.message
    );
}

#[tokio::test]
async fn test_tool_call_internal_error() {
    // Verify that a panicking execute closure produces MCP_INTERNAL_ERROR.
    // dispatch_tool_call_spawned runs the call in a tokio::spawn and maps
    // JoinError (panic) → MCP_INTERNAL_ERROR (AC-E-INTERNAL, §4.7).
    let panicking_cmd = Command {
        id: Arc::from("panic-cmd"),
        spec: Arc::new(CommandSpec {
            summary: "Panicking command",
            ..Default::default()
        }),
        validator: None,
        expose_mcp: false,
        expose_chat: true,
        meta: None,
        visibility: None,
        execute: Arc::new(|_ctx, _args: HashMap<String, ArgValue>| {
            Box::pin(async move {
                panic!("intentional panic for MCP_INTERNAL_ERROR test");
                #[allow(unreachable_code)]
                Ok(())
            })
        }),
    };

    let mut registry = CommandRegistry::new();
    registry.register(panicking_cmd);
    let tool_registry = Arc::new(McpToolRegistry::from_command_registry(&registry, "myapp"));

    let result = dispatch_tool_call_spawned(
        tool_registry,
        "myapp_panic-cmd".to_string(),
        None,
        McpTransportKind::Http,
    )
    .await;
    assert!(result.is_err());
    let err = result.unwrap_err();
    assert!(
        err.message.starts_with("MCP_INTERNAL_ERROR:"),
        "expected MCP_INTERNAL_ERROR, got: {}",
        err.message
    );
}

#[derive(Debug)]
struct DenyGate;

#[async_trait::async_trait]
impl ExecutionGate for DenyGate {
    async fn before_execute(
        &self,
        _cmd: &Command,
        _args: &HashMap<String, ArgValue>,
        _tier: CommandRiskTier,
    ) -> Result<(), GateError> {
        Err(GateError::Denied {
            reason: "blocked by test gate".to_string(),
        })
    }
}

#[derive(Debug)]
struct FailGate;

#[async_trait::async_trait]
impl ExecutionGate for FailGate {
    async fn before_execute(
        &self,
        _cmd: &Command,
        _args: &HashMap<String, ArgValue>,
        _tier: CommandRiskTier,
    ) -> Result<(), GateError> {
        Err(GateError::Failed {
            reason: "gate crashed".to_string(),
        })
    }
}

#[tokio::test]
async fn test_gate_denied_maps_to_mcp_tool_denied() {
    let cmd = make_cmd("hello");
    let mut registry = CommandRegistry::new();
    registry.register(cmd);
    let tool_registry =
        McpToolRegistry::from_command_registry(&registry, "myapp").with_gate(Arc::new(DenyGate));

    let err = dispatch_tool_call(&tool_registry, "myapp_hello", None, McpTransportKind::Http)
        .await
        .unwrap_err();

    assert_eq!(err.code, rmcp::model::ErrorCode(-32005));
    assert!(
        err.message.starts_with("MCP_TOOL_DENIED:"),
        "got: {}",
        err.message
    );
}

#[tokio::test]
async fn test_gate_failed_maps_to_mcp_tool_gate_failed() {
    let cmd = make_cmd("hello");
    let mut registry = CommandRegistry::new();
    registry.register(cmd);
    let tool_registry =
        McpToolRegistry::from_command_registry(&registry, "myapp").with_gate(Arc::new(FailGate));

    let err = dispatch_tool_call(&tool_registry, "myapp_hello", None, McpTransportKind::Http)
        .await
        .unwrap_err();

    assert_eq!(err.code, rmcp::model::ErrorCode(-32006));
    assert!(
        err.message.starts_with("MCP_TOOL_GATE_FAILED:"),
        "got: {}",
        err.message
    );
}
