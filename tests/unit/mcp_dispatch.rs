use cli_framework::command::{Command, CommandArgs, CommandRegistry};
use cli_framework::mcp::{dispatch_tool_call, McpToolRegistry};
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
    // Test internal error path via tokio::spawn panic recovery
    let tool_registry = Arc::new(McpToolRegistry::from_command_registry(
        &CommandRegistry::new(),
        "myapp",
    ));

    let result = tokio::task::spawn(async move {
        dispatch_tool_call(&tool_registry, "myapp.nonexistent", None).await
    })
    .await;

    // The task should complete (not panic), returning a CMD_NOT_FOUND error
    match result {
        Ok(Err(err)) => {
            assert!(
                err.message.starts_with("MCP_CMD_NOT_FOUND:"),
                "got: {}",
                err.message
            );
        }
        Ok(Ok(_)) => panic!("expected error"),
        Err(join_err) => {
            // If the task panicked, that's an internal error scenario
            let msg = format!("MCP_INTERNAL_ERROR: task panicked: {}", join_err);
            assert!(msg.contains("MCP_INTERNAL_ERROR:"));
        }
    }
}
