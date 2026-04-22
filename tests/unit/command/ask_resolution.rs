use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use cli_framework::app::AppContext;
use cli_framework::command::{Command, CommandArgs, CommandRegistry};
use cli_framework::llm::{CommandMetadata, CommandResolution, LlmProvider};

struct MockLlmProvider {
    resolution: Arc<Mutex<Option<anyhow::Result<CommandResolution>>>>,
    captured_query: Arc<Mutex<String>>,
    captured_commands: Arc<Mutex<Vec<CommandMetadata>>>,
}

impl MockLlmProvider {
    fn new() -> Self {
        Self {
            resolution: Arc::new(Mutex::new(None)),
            captured_query: Arc::new(Mutex::new(String::new())),
            captured_commands: Arc::new(Mutex::new(Vec::new())),
        }
    }

    fn with_resolution(resolution: CommandResolution) -> Self {
        let s = Self::new();
        *s.resolution.lock().unwrap() = Some(Ok(resolution));
        s
    }

    fn with_error(msg: &str) -> Self {
        let s = Self::new();
        *s.resolution.lock().unwrap() = Some(Err(anyhow::anyhow!("{}", msg)));
        s
    }
}

#[async_trait]
impl LlmProvider for MockLlmProvider {
    async fn resolve_command(
        &self,
        query: &str,
        commands: &[CommandMetadata],
    ) -> anyhow::Result<CommandResolution> {
        *self.captured_query.lock().unwrap() = query.to_string();
        *self.captured_commands.lock().unwrap() = commands.to_vec();
        self.resolution
            .lock()
            .unwrap()
            .take()
            .unwrap_or(Err(anyhow::anyhow!("No resolution configured")))
    }
}

struct NoopContext;
impl AppContext for NoopContext {}

fn create_test_registry_with_commands() -> CommandRegistry {
    let mut registry = CommandRegistry::new();
    registry.register(Command {
        id: "deploy",
        summary: "Deploy application",
        syntax: Some("deploy --env <env>"),
        category: Some("deployment"),
        execute: Arc::new(|_ctx, _args| Box::pin(async move { Ok(()) })),
    });
    registry.register(Command {
        id: "status",
        summary: "Show status",
        syntax: Some("status"),
        category: Some("monitoring"),
        execute: Arc::new(|_ctx, _args| Box::pin(async move { Ok(()) })),
    });
    registry
}

fn make_resolution(command_id: &str, confidence: f32) -> CommandResolution {
    CommandResolution {
        command_id: command_id.to_string(),
        args: CommandArgs::default(),
        confidence,
        reasoning: Some("test reasoning".to_string()),
    }
}

#[tokio::test]
async fn test_ask_positional_query_calls_resolve() {
    let mock = MockLlmProvider::with_resolution(make_resolution("status", 0.95));
    let captured_query = mock.captured_query.clone();
    let captured_commands = mock.captured_commands.clone();

    let registry = create_test_registry_with_commands();
    let ask_cmd = cli_framework::command::create_ask_command(Arc::new(mock), Arc::new(registry));

    let mut ctx = NoopContext;
    let mut named1 = HashMap::new();
    named1.insert("yes".to_string(), "true".to_string());
    let args = CommandArgs {
        positional: vec!["show".to_string(), "status".to_string()],
        named: named1,
    };

    let result = (ask_cmd.execute)(&mut ctx, args).await;

    assert!(result.is_ok());
    assert_eq!(*captured_query.lock().unwrap(), "show status");
    assert!(!captured_commands.lock().unwrap().is_empty());
}

#[tokio::test]
async fn test_ask_named_query_calls_resolve() {
    let mock = MockLlmProvider::with_resolution(make_resolution("status", 0.9));
    let captured_query = mock.captured_query.clone();

    let registry = create_test_registry_with_commands();
    let ask_cmd = cli_framework::command::create_ask_command(Arc::new(mock), Arc::new(registry));

    let mut ctx = NoopContext;
    let mut named = HashMap::new();
    named.insert("query".to_string(), "show status".to_string());
    named.insert("yes".to_string(), "true".to_string());
    let args = CommandArgs {
        positional: vec![],
        named,
    };

    let result = (ask_cmd.execute)(&mut ctx, args).await;

    assert!(result.is_ok());
    assert_eq!(*captured_query.lock().unwrap(), "show status");
}

#[tokio::test]
async fn test_ask_no_query_returns_ok() {
    let mock = MockLlmProvider::new();

    let registry = create_test_registry_with_commands();
    let ask_cmd = cli_framework::command::create_ask_command(Arc::new(mock), Arc::new(registry));

    let mut ctx = NoopContext;
    let args = CommandArgs {
        positional: vec![],
        named: HashMap::new(),
    };

    let result = (ask_cmd.execute)(&mut ctx, args).await;
    assert!(result.is_ok());
}

#[tokio::test]
async fn test_ask_excludes_self_from_metadata() {
    let mock = MockLlmProvider::with_resolution(make_resolution("status", 0.9));
    let captured_commands = mock.captured_commands.clone();

    let registry = create_test_registry_with_commands();
    let ask_cmd = cli_framework::command::create_ask_command(Arc::new(mock), Arc::new(registry));

    let mut ctx = NoopContext;
    let mut named2 = HashMap::new();
    named2.insert("yes".to_string(), "true".to_string());
    let args = CommandArgs {
        positional: vec!["status".to_string()],
        named: named2,
    };

    let _ = (ask_cmd.execute)(&mut ctx, args).await;

    let commands = captured_commands.lock().unwrap();
    let has_ask = commands.iter().any(|m| m.id == "ask");
    assert!(!has_ask, "ask should be excluded from metadata");
    assert_eq!(commands.len(), 2);
}

#[tokio::test]
async fn test_ask_dispatches_resolved_command() {
    let executed: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
    let executed_clone = executed.clone();

    let mut registry = CommandRegistry::new();
    registry.register(Command {
        id: "greet",
        summary: "Greet someone",
        syntax: None,
        category: None,
        execute: Arc::new(move |_ctx, args| {
            let executed_clone = executed_clone.clone();
            Box::pin(async move {
                let name = args.positional.get(0).cloned().unwrap_or_default();
                executed_clone
                    .lock()
                    .unwrap()
                    .push(format!("greet:{}", name));
                Ok(())
            })
        }),
    });

    let resolution = CommandResolution {
        command_id: "greet".to_string(),
        args: CommandArgs {
            positional: vec!["Alice".to_string()],
            named: HashMap::new(),
        },
        confidence: 0.95,
        reasoning: None,
    };

    let mock = MockLlmProvider::with_resolution(resolution);

    let ask_cmd = cli_framework::command::create_ask_command(Arc::new(mock), Arc::new(registry));

    let mut ctx = NoopContext;
    let mut named = HashMap::new();
    named.insert("yes".to_string(), "true".to_string());
    let args = CommandArgs {
        positional: vec!["greet".to_string(), "Alice".to_string()],
        named,
    };

    let result = (ask_cmd.execute)(&mut ctx, args).await;

    assert!(result.is_ok());
    let executed_cmds = executed.lock().unwrap();
    assert_eq!(executed_cmds.len(), 1);
    assert_eq!(executed_cmds[0], "greet:Alice");
}

#[tokio::test]
async fn test_ask_unknown_command_returns_error() {
    let resolution = CommandResolution {
        command_id: "nonexistent".to_string(),
        args: CommandArgs::default(),
        confidence: 0.8,
        reasoning: None,
    };
    let mock = MockLlmProvider::with_resolution(resolution);

    let registry = CommandRegistry::new();
    let ask_cmd = cli_framework::command::create_ask_command(Arc::new(mock), Arc::new(registry));

    let mut ctx = NoopContext;
    let mut named3 = HashMap::new();
    named3.insert("yes".to_string(), "true".to_string());
    let args = CommandArgs {
        positional: vec!["do".to_string(), "something".to_string()],
        named: named3,
    };

    let result = (ask_cmd.execute)(&mut ctx, args).await;

    assert!(result.is_err());
    assert!(result.unwrap_err().to_string().contains("not found"),);
}

#[tokio::test]
async fn test_ask_recursive_ask_returns_error() {
    let resolution = CommandResolution {
        command_id: "ask".to_string(),
        args: CommandArgs::default(),
        confidence: 0.5,
        reasoning: None,
    };
    let mock = MockLlmProvider::with_resolution(resolution);

    let registry = CommandRegistry::new();
    let ask_cmd = cli_framework::command::create_ask_command(Arc::new(mock), Arc::new(registry));

    let mut ctx = NoopContext;
    let args = CommandArgs {
        positional: vec!["ask".to_string(), "something".to_string()],
        named: HashMap::new(),
    };

    let result = (ask_cmd.execute)(&mut ctx, args).await;
    assert!(result.is_err());
    let err = result.unwrap_err().to_string();
    assert!(
        err.contains("Recursive ask invocation"),
        "Expected recursive error, got: {}",
        err
    );
}

#[tokio::test]
async fn test_ask_provider_error_returns_error() {
    let mock = MockLlmProvider::with_error("API rate limit exceeded");

    let registry = CommandRegistry::new();
    let ask_cmd = cli_framework::command::create_ask_command(Arc::new(mock), Arc::new(registry));

    let mut ctx = NoopContext;
    let args = CommandArgs {
        positional: vec!["deploy".to_string()],
        named: HashMap::new(),
    };

    let result = (ask_cmd.execute)(&mut ctx, args).await;
    assert!(result.is_err());
    assert!(result
        .unwrap_err()
        .to_string()
        .contains("LLM resolution failed"),);
}

#[tokio::test]
async fn test_ask_yes_flag_skips_confirmation() {
    let mock = MockLlmProvider::with_resolution(make_resolution("status", 0.9));

    let registry = create_test_registry_with_commands();
    let ask_cmd = cli_framework::command::create_ask_command(Arc::new(mock), Arc::new(registry));

    let mut ctx = NoopContext;
    let mut named = HashMap::new();
    named.insert("yes".to_string(), "true".to_string());
    let args = CommandArgs {
        positional: vec!["status".to_string()],
        named,
    };

    let result = (ask_cmd.execute)(&mut ctx, args).await;
    assert!(result.is_ok());
}

#[tokio::test]
async fn test_app_without_llm_has_no_ask_in_help() {
    use cli_framework::app::AppBuilder;

    struct DummyCtx;
    impl AppContext for DummyCtx {}

    let app = AppBuilder::new()
        .register_command(Command {
            id: "hello",
            summary: "Say hello",
            syntax: None,
            category: None,
            execute: Arc::new(|_ctx, _args| Box::pin(async move { Ok(()) })),
        })
        .build(DummyCtx)
        .unwrap();

    let help = app.render_help();
    assert!(
        !help.contains("  ask "),
        "ask command should not appear in help without LLM: {}",
        help
    );
}
