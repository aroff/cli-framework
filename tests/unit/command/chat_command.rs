use cli_framework::app::AppContext;
use cli_framework::command::chat::{
    ChatToolCallOptions, CommandsAsToolsExecutor, HostToolExecutor, CHAT_ARG_VALIDATION_FAILED,
    CHAT_COMMAND_EXECUTION_FAILED, CHAT_TOOL_NOT_FOUND,
};
use cli_framework::command::{Command, CommandRegistry};
use cli_framework::security::command_risk::CommandRiskPolicy;
use cli_framework::spec::arg_spec::{ArgKind, ArgSpec, ArgValueType, Cardinality};
use cli_framework::spec::command_tree::CommandPath;
use cli_framework::spec::command_tree::CommandSpec;
use cli_framework::spec::value::ArgValue;
use std::collections::HashMap;
use std::sync::Arc;

#[derive(Default)]
struct Ctx;

impl AppContext for Ctx {}

fn make_spec_command(id: &'static str) -> Command {
    Command {
        id,
        summary: "test",
        syntax: None,
        category: None,
        spec: Some(Arc::new(CommandSpec {
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
            }],
            ..Default::default()
        })),
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
        execute: Arc::new(|_ctx, args| {
            Box::pin(async move {
                let name = args.named.get("name").cloned().unwrap_or_default();
                assert!(!name.is_empty());
                Ok(())
            })
        }),
    }
}

#[tokio::test]
async fn tool_names_match_mcp_convention() {
    let mut registry = CommandRegistry::new();
    let path = CommandPath::new(&["group", "do"]).unwrap();
    registry
        .register_at(&path, make_spec_command("do"))
        .unwrap();

    let exec =
        CommandsAsToolsExecutor::new(&registry, "myapp", CommandRiskPolicy::default()).unwrap();

    let tools = exec.list_tools();
    let names: Vec<String> = tools.into_iter().map(|t| t.name).collect();
    assert!(names.iter().any(|n| n == "myapp.group.do"));
}

#[tokio::test]
async fn unknown_tool_returns_chat_tool_not_found() {
    let registry = CommandRegistry::new();
    let exec =
        CommandsAsToolsExecutor::new(&registry, "myapp", CommandRiskPolicy::default()).unwrap();
    let mut ctx = Ctx::default();

    let err = exec
        .call_tool(
            "myapp.missing",
            serde_json::json!({}),
            &mut ctx,
            &ChatToolCallOptions {
                yolo: true,
                interactive: false,
                ailoop_client: None,
            },
        )
        .await
        .unwrap_err();
    assert!(err.to_string().contains(CHAT_TOOL_NOT_FOUND));
}

#[tokio::test]
async fn invalid_typed_args_returns_chat_arg_validation_failed() {
    let mut registry = CommandRegistry::new();
    registry.register(make_spec_command("do"));

    let exec =
        CommandsAsToolsExecutor::new(&registry, "myapp", CommandRiskPolicy::default()).unwrap();
    let mut ctx = Ctx::default();

    let err = exec
        .call_tool(
            "myapp.do",
            serde_json::json!({}),
            &mut ctx,
            &ChatToolCallOptions {
                yolo: true,
                interactive: false,
                ailoop_client: None,
            },
        )
        .await
        .unwrap_err();
    assert!(err.to_string().contains(CHAT_ARG_VALIDATION_FAILED));
}

#[tokio::test]
async fn command_execution_error_surfaces_chat_command_execution_failed() {
    let mut registry = CommandRegistry::new();
    registry.register(Command {
        id: "boom",
        summary: "boom",
        syntax: None,
        category: None,
        spec: None,
        validator: None,
        expose_mcp: false,
        execute: Arc::new(|_ctx, _args| Box::pin(async { Err(anyhow::anyhow!("nope")) })),
    });

    let exec =
        CommandsAsToolsExecutor::new(&registry, "myapp", CommandRiskPolicy::default()).unwrap();
    let mut ctx = Ctx::default();

    let err = exec
        .call_tool(
            "myapp.boom",
            serde_json::json!({}),
            &mut ctx,
            &ChatToolCallOptions {
                yolo: true,
                interactive: false,
                ailoop_client: None,
            },
        )
        .await
        .unwrap_err();
    assert!(err.to_string().contains(CHAT_COMMAND_EXECUTION_FAILED));
}
