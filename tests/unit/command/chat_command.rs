use cli_framework::command::chat::host_tool_adapter::McpHostToolAdapter;
use cli_framework::command::chat::{
    ChatToolCallOptions, CHAT_ARG_VALIDATION_FAILED, CHAT_COMMAND_EXECUTION_FAILED,
    CHAT_TOOL_NOT_FOUND,
};
use cli_framework::command::{Command, CommandRegistry};
use cli_framework::mcp::{McpToolExportPolicy, McpToolRegistry};
use cli_framework::security::command_risk::CommandRiskPolicy;
use cli_framework::spec::arg_spec::{ArgKind, ArgSpec, ArgValueType, Cardinality};
use cli_framework::spec::command_tree::CommandPath;
use cli_framework::spec::command_tree::CommandSpec;
use cli_framework::spec::value::ArgValue;
use std::collections::HashMap;
use std::sync::Arc;

fn make_spec_command(id: &'static str) -> Command {
    Command {
        id: Arc::from(id),
        spec: Arc::new(CommandSpec {
            summary: "test",
            args: vec![ArgSpec {
                name: "name",
                kind: ArgKind::Option,
                short: None,
                long: Some("name"),
                value_type: ArgValueType::String,
                cardinality: Cardinality::Required,
                default: None,
                conflicts_with: vec![],
                requires: vec![],
                help: "name",
                ..Default::default()
            }],
            ..Default::default()
        }),
        validator: Some(Arc::new(|typed: &HashMap<String, ArgValue>| {
            if let Some(ArgValue::Str(s)) = typed.get("name") {
                if s.is_empty() {
                    return vec![cli_framework::parser::diagnostic::Diagnostic {
                        code: "E999",
                        category: cli_framework::parser::diagnostic::DiagnosticCategory::Spec,
                        message: "name must not be empty".to_string(),
                        suggestion: None,
                        span: None,
                    }];
                }
            }
            vec![]
        })),
        expose_mcp: false,
        expose_chat: true,
        ui: None,
        visibility: None,
        execute: Arc::new(|_ctx, args| {
            Box::pin(async move {
                let name = args
                    .get("name")
                    .and_then(|v| {
                        if let ArgValue::Str(s) = v {
                            Some(s.clone())
                        } else {
                            None
                        }
                    })
                    .unwrap_or_default();
                assert!(!name.is_empty());
                Ok(())
            })
        }),
    }
}

fn make_registry_executor(registry: &CommandRegistry) -> McpToolRegistry {
    McpToolRegistry::from_command_registry_with_policy(
        registry,
        "myapp",
        McpToolExportPolicy::AllCommands,
    )
    .with_risk_policy(CommandRiskPolicy::default())
}

fn default_opts() -> ChatToolCallOptions {
    ChatToolCallOptions {
        yolo: true,
        interactive: false,
        ailoop_client: None,
    }
}

fn make_adapter(exec: Arc<McpToolRegistry>) -> Arc<McpHostToolAdapter> {
    Arc::new(McpHostToolAdapter::new(exec, default_opts()))
}

#[tokio::test]
async fn tool_names_match_mcp_convention() {
    let mut registry = CommandRegistry::new();
    let path = CommandPath::new(&["group", "do"]).unwrap();
    registry
        .register_at(&path, make_spec_command("do"))
        .unwrap();

    let exec = make_registry_executor(&registry);
    let tools = exec.list_tools();
    let names: Vec<String> = tools.into_iter().map(|t| t.name).collect();
    assert!(names.iter().any(|n| n == "myapp_group_do"));
}

#[tokio::test]
async fn completion_command_not_exposed_as_tool() {
    let mut registry = CommandRegistry::new();
    registry.register(Command {
        id: Arc::from("completion"),
        spec: Arc::new(CommandSpec {
            summary: "shell completion",
            ..Default::default()
        }),
        validator: None,
        expose_mcp: false,
        expose_chat: true,
        ui: None,
        visibility: None,
        execute: Arc::new(|_ctx, _args| Box::pin(async { Ok(()) })),
    });
    registry.register(make_spec_command("deploy"));

    let exec = make_registry_executor(&registry);
    let tools = exec.list_tools();
    let names: Vec<String> = tools.into_iter().map(|t| t.name).collect();
    assert!(
        !names.iter().any(|n| n.ends_with("_completion")),
        "completion must not appear in tool list, got: {:?}",
        names
    );
}

#[tokio::test]
async fn expose_mcp_only_policy_filters_commands() {
    let mut registry = CommandRegistry::new();
    registry.register(Command {
        id: Arc::from("private_cmd"),
        spec: Arc::new(CommandSpec {
            summary: "not exported",
            ..Default::default()
        }),
        validator: None,
        expose_mcp: false,
        expose_chat: true,
        ui: None,
        visibility: None,
        execute: Arc::new(|_ctx, _args| Box::pin(async { Ok(()) })),
    });
    registry.register(Command {
        id: Arc::from("public_cmd"),
        spec: Arc::new(CommandSpec {
            summary: "exported",
            ..Default::default()
        }),
        validator: None,
        expose_mcp: true,
        expose_chat: true,
        ui: None,
        visibility: None,
        execute: Arc::new(|_ctx, _args| Box::pin(async { Ok(()) })),
    });

    let exec = McpToolRegistry::from_command_registry_with_policy(
        &registry,
        "myapp",
        McpToolExportPolicy::ExposeMcpOnly,
    )
    .with_risk_policy(CommandRiskPolicy::default());

    let tools = exec.list_tools();
    let names: Vec<String> = tools.into_iter().map(|t| t.name).collect();
    assert!(
        names.iter().any(|n| n == "myapp_public_cmd"),
        "public_cmd should be exposed"
    );
    assert!(
        !names.iter().any(|n| n == "myapp_private_cmd"),
        "private_cmd must not be exposed with ExposeMcpOnly"
    );
}

#[tokio::test]
async fn unknown_tool_returns_chat_tool_not_found() {
    let registry = CommandRegistry::new();
    let exec = Arc::new(make_registry_executor(&registry));
    let adapter = make_adapter(exec);

    let err = tokio::task::spawn_blocking(move || {
        adapter.call_tool("myapp_missing", serde_json::json!({}))
    })
    .await
    .unwrap()
    .unwrap_err();
    assert!(err.to_string().contains(CHAT_TOOL_NOT_FOUND));
}

#[tokio::test]
async fn invalid_typed_args_returns_chat_arg_validation_failed() {
    let mut registry = CommandRegistry::new();
    registry.register(make_spec_command("do"));

    let exec = Arc::new(make_registry_executor(&registry));
    let adapter = make_adapter(exec);

    let err =
        tokio::task::spawn_blocking(move || adapter.call_tool("myapp_do", serde_json::json!({})))
            .await
            .unwrap()
            .unwrap_err();
    assert!(err.to_string().contains(CHAT_ARG_VALIDATION_FAILED));
}

#[tokio::test]
async fn command_execution_error_surfaces_chat_command_execution_failed() {
    let mut registry = CommandRegistry::new();
    registry.register(Command {
        id: Arc::from("boom"),
        spec: Arc::new(CommandSpec {
            summary: "boom",
            ..Default::default()
        }),
        validator: None,
        expose_mcp: false,
        expose_chat: true,
        ui: None,
        visibility: None,
        execute: Arc::new(|_ctx, _args| Box::pin(async { Err(anyhow::anyhow!("nope")) })),
    });

    let exec = Arc::new(make_registry_executor(&registry));
    let adapter = make_adapter(exec);

    let err =
        tokio::task::spawn_blocking(move || adapter.call_tool("myapp_boom", serde_json::json!({})))
            .await
            .unwrap()
            .unwrap_err();
    assert!(err.to_string().contains(CHAT_COMMAND_EXECUTION_FAILED));
}

/// U1: call_tool for a command that outputs via framework_println captures the output.
#[tokio::test]
async fn call_tool_captures_framework_println_output() {
    let mut registry = CommandRegistry::new();
    registry.register(Command {
        id: Arc::from("greet"),
        spec: Arc::new(CommandSpec {
            summary: "greet",
            ..Default::default()
        }),
        validator: None,
        expose_mcp: false,
        expose_chat: true,
        ui: None,
        visibility: None,
        execute: Arc::new(|ctx, _args| {
            Box::pin(async move {
                ctx.framework_println("json-output");
                Ok(())
            })
        }),
    });

    let exec = Arc::new(make_registry_executor(&registry));
    let adapter = make_adapter(exec);

    let output = tokio::task::spawn_blocking(move || {
        adapter.call_tool("myapp_greet", serde_json::json!({}))
    })
    .await
    .unwrap()
    .unwrap();
    assert!(output.contains("json-output"));
}

/// U7: list_tools returns descriptors with underscore-separated names.
#[tokio::test]
async fn list_tools_returns_underscore_separated_names_and_parameters() {
    let mut registry = CommandRegistry::new();
    registry.register(make_spec_command("deploy"));

    let exec = Arc::new(make_registry_executor(&registry));
    let adapter = McpHostToolAdapter::new(Arc::clone(&exec), default_opts());

    let tools = adapter.list_tools();
    assert!(!tools.is_empty());
    for tool in &tools {
        assert!(
            !tool.name.contains('/'),
            "tool name must use underscores, got: {}",
            tool.name
        );
        // parameters field must be populated (JSON Schema object)
        assert!(
            tool.parameters.is_object(),
            "parameters must be a JSON object"
        );
    }
}
