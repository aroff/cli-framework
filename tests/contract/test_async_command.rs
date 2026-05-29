//! Contract tests for async Command execution
//!
//! Verifies that Command implementations correctly execute async operations.

use std::collections::HashMap;
use std::sync::Arc;
use tokio::time::{sleep, Duration};
use cli_framework::app::AppContext;
use cli_framework::command::{Command, CommandArgs};

/// Test context for async Command tests
struct TestContext {
    #[allow(dead_code)]
    command_executed: bool,
    #[allow(dead_code)]
    result: String,
}

impl AppContext for TestContext {}

/// Helper to create an async command
/// Note: This will fail to compile until Command::execute is converted to async
fn create_async_command(id: &'static str, summary: &'static str) -> Command {
    Command {
        id,
        summary,
        syntax: None,
        category: None,
        spec: None,
        validator: None,
        expose_mcp: false,
        execute: Arc::new(|_ctx: &mut dyn AppContext, _args: CommandArgs| {
            Box::pin(async move {
                sleep(Duration::from_millis(10)).await;
                Ok(())
            })
        }),
    };

    let mut ctx = TestContext {
        command_executed: false,
        result: String::new(),
    };

    let args = CommandArgs {
        positional: vec![],
        named: HashMap::new(),
        ..Default::default()
    };

    let start = std::time::Instant::now();
    let result = (command.execute)(&mut ctx, args).await;
    let elapsed = start.elapsed();

    assert!(elapsed >= Duration::from_millis(10));
    assert!(result.is_ok());
}

#[tokio::test]
async fn test_async_command_handles_errors() {
    let command = Command {
        id: "test.error",
        summary: "Test error handling",
        syntax: None,
        category: None,
        spec: None,
        validator: None,
        expose_mcp: false,
        execute: Arc::new(|_ctx: &mut dyn AppContext, _args: CommandArgs| {
            Box::pin(async move {
                sleep(Duration::from_millis(5)).await;
                anyhow::bail!("Command failed");
            })
        }),
    };

    let mut ctx = TestContext {
        command_executed: false,
        result: String::new(),
    };

    let args = CommandArgs {
        positional: vec![],
        named: HashMap::new(),
    };

    let result = (command.execute)(&mut ctx, args).await;
    assert!(result.is_err());
    assert!(result.unwrap_err().to_string().contains("Command failed"));
}

#[tokio::test]
async fn test_async_command_can_access_args() {
    // This test will fail to compile until Command::execute is converted to async
    let command = Command {
        id: "test.args",
        summary: "Test command args",
        syntax: None,
        category: None,
        spec: None,
        validator: None,
        expose_mcp: false,
        execute: Arc::new(|_ctx: &mut dyn AppContext, args: CommandArgs| {
            Box::pin(async move {
                assert_eq!(args.positional.len(), 2);
                assert_eq!(args.positional[0], "arg1");
                assert_eq!(args.positional[1], "arg2");
                assert_eq!(args.named.get("key"), Some(&"value".to_string()));
                Ok(())
            })
        }),
    };

    let mut ctx = TestContext {
        command_executed: false,
        result: String::new(),
    };

    let mut named = HashMap::new();
    named.insert("key".to_string(), "value".to_string());

    let args = CommandArgs {
        positional: vec!["arg1".to_string(), "arg2".to_string()],
        named,
        ..Default::default()
    };

    let result = (command.execute)(&mut ctx, args).await;
    assert!(result.is_ok());
}
