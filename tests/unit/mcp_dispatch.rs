use cli_framework::command::{Command, CommandArgs, CommandRegistry};
use cli_framework::mcp::{dispatch_tool_call, dispatch_tool_call_spawned, McpToolRegistry};
use cli_framework::spec::arg_spec::{ArgKind, ArgSpec, ArgValueType, Cardinality};
use cli_framework::spec::command_tree::CommandSpec;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

fn noop_execute() -> Arc<
    dyn Fn(
            &mut dyn cli_framework::app::AppContext,
            CommandArgs,
        ) -> Pin<Box<dyn Future<Output = anyhow::Result<()>> + Send>>
        + Send
        + Sync,
> {
    Arc::new(|_ctx, _args| Box::pin(async { Ok(()) }))
}

fn failing_execute() -> Arc<
    dyn Fn(
            &mut dyn cli_framework::app::AppContext,
            CommandArgs,
        ) -> Pin<Box<dyn Future<Output = anyhow::Result<()>> + Send>>
        + Send
        + Sync,
> {
    Arc::new(|_ctx, _args| Box::pin(async { Err(anyhow::anyhow!("command execution failed")) }))
}

fn make_cmd(id: &'static str) -> Command {
    Command {
        id,
        summary: "test command",
        syntax: None,
        category: None,
        spec: None,
        validator: None,
        execute: noop_execute(),
    }
}

fn make_registry_with_cmd(id: &'static str, cmd: Command) -> McpToolRegistry {
    let mut registry = CommandRegistry::new();
    registry.register(cmd);
    McpToolRegistry::from_command_registry(&registry, "myapp")
}

#[tokio::test]
async fn test_tool_call_success() {
    let cmd = make_cmd("hello");
    let tool_registry = make_registry_with_cmd("hello", cmd);

    let result = dispatch_tool_call(&tool_registry, "myapp.hello", None).await;
    assert!(result.is_ok());
    let call_result = result.unwrap();
    assert_eq!(call_result.is_error, Some(false));
}

#[tokio::test]
async fn test_tool_call_cmd_not_found() {
    let tool_registry = McpToolRegistry::from_command_registry(&CommandRegistry::new(), "myapp");

    let result = dispatch_tool_call(&tool_registry, "myapp.nonexistent", None).await;
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
        }],
        ..Default::default()
    };
    let cmd = Command {
        id: "test-cmd",
        summary: "test command",
        syntax: None,
        category: None,
        spec: Some(Arc::new(spec)),
        validator: None,
        execute: noop_execute(),
    };

    let mut registry = CommandRegistry::new();
    registry.register(cmd);
    let tool_registry = McpToolRegistry::from_command_registry(&registry, "myapp");

    // Call without required arg
    let result = dispatch_tool_call(&tool_registry, "myapp.test-cmd", None).await;
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
        id: "fail-cmd",
        summary: "failing command",
        syntax: None,
        category: None,
        spec: None,
        validator: None,
        execute: failing_execute(),
    };

    let tool_registry = make_registry_with_cmd("fail-cmd", cmd);

    let result = dispatch_tool_call(&tool_registry, "myapp.fail-cmd", None).await;
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
        id: "panic-cmd",
        summary: "Panicking command",
        syntax: None,
        category: None,
        spec: None,
        validator: None,
        execute: Arc::new(|_ctx, _args| {
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

    let result =
        dispatch_tool_call_spawned(tool_registry, "myapp.panic-cmd".to_string(), None).await;
    assert!(result.is_err());
    let err = result.unwrap_err();
    assert!(
        err.message.starts_with("MCP_INTERNAL_ERROR:"),
        "expected MCP_INTERNAL_ERROR, got: {}",
        err.message
    );
}
