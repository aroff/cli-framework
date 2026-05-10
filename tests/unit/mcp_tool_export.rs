use cli_framework::command::{Command, CommandArgs, CommandRegistry};
use cli_framework::mcp::{McpToolExportPolicy, McpToolRegistry};
use cli_framework::spec::arg_spec::{ArgKind, ArgSpec, ArgValueType, Cardinality};
use cli_framework::spec::command_tree::{CommandPath, CommandSpec};
use insta;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

fn noop_execute() -> Arc<
    dyn for<'a> Fn(
            &'a mut dyn cli_framework::app::AppContext,
            CommandArgs,
        ) -> Pin<Box<dyn Future<Output = anyhow::Result<()>> + Send + 'a>>
        + Send
        + Sync,
> {
    Arc::new(|_ctx, _args| Box::pin(async { Ok(()) }))
}

fn make_cmd(id: &'static str, summary: &'static str) -> Command {
    Command {
        id,
        summary,
        syntax: None,
        category: None,
        spec: None,
        validator: None,
        expose_mcp: false,
        execute: noop_execute(),
    }
}

fn make_cmd_with_spec(id: &'static str, summary: &'static str, spec: CommandSpec) -> Command {
    Command {
        id,
        summary,
        syntax: None,
        category: None,
        spec: Some(Arc::new(spec)),
        validator: None,
        expose_mcp: false,
        execute: noop_execute(),
    }
}

#[test]
fn test_tool_list_includes_all_commands() {
    let mut registry = CommandRegistry::new();
    registry.register(make_cmd("deploy", "Deploy app"));
    registry.register(make_cmd("status", "Show status"));
    registry.register(make_cmd("logs", "Show logs"));

    let tool_registry = McpToolRegistry::from_command_registry(&registry, "myapp");
    let tools = tool_registry.list_tools();
    assert_eq!(tools.len(), 3);
}

#[test]
fn test_tool_name_format() {
    let mut registry = CommandRegistry::new();
    registry.register(make_cmd("deploy", "Deploy app"));

    let tool_registry = McpToolRegistry::from_command_registry(&registry, "myapp");
    let tools = tool_registry.list_tools();
    assert_eq!(tools.len(), 1);
    assert_eq!(tools[0].name, "myapp.deploy");
}

#[test]
fn test_required_arg_in_schema_required_array() {
    let spec = CommandSpec {
        args: vec![ArgSpec {
            name: "env",
            kind: ArgKind::Option,
            short: None,
            long: None,
            value_type: ArgValueType::String,
            cardinality: Cardinality::Required,
            default: None,
            conflicts_with: vec![],
            requires: vec![],
            help: "Environment",
        }],
        ..Default::default()
    };
    let mut registry = CommandRegistry::new();
    registry.register(make_cmd_with_spec("deploy", "Deploy app", spec));

    let tool_registry = McpToolRegistry::from_command_registry(&registry, "myapp");
    let tools = tool_registry.list_tools();
    assert_eq!(tools.len(), 1);

    let schema = &tools[0].input_schema;
    let required = schema["required"]
        .as_array()
        .expect("required array must exist");
    assert!(required.iter().any(|v| v.as_str() == Some("env")));
}

#[test]
fn test_optional_arg_not_in_required_array() {
    let spec = CommandSpec {
        args: vec![ArgSpec {
            name: "verbose",
            kind: ArgKind::Flag,
            short: None,
            long: None,
            value_type: ArgValueType::Bool,
            cardinality: Cardinality::Optional,
            default: None,
            conflicts_with: vec![],
            requires: vec![],
            help: "Verbose mode",
        }],
        ..Default::default()
    };
    let mut registry = CommandRegistry::new();
    registry.register(make_cmd_with_spec("status", "Show status", spec));

    let tool_registry = McpToolRegistry::from_command_registry(&registry, "myapp");
    let tools = tool_registry.list_tools();
    let schema = &tools[0].input_schema;

    if let Some(required) = schema.get("required").and_then(|v| v.as_array()) {
        assert!(!required.iter().any(|v| v.as_str() == Some("verbose")));
    }
}

#[test]
fn test_spec_less_command_permissive_schema() {
    let mut registry = CommandRegistry::new();
    registry.register(make_cmd("hello", "Say hello"));

    let tool_registry = McpToolRegistry::from_command_registry(&registry, "myapp");
    let tools = tool_registry.list_tools();
    let schema = &tools[0].input_schema;

    assert_eq!(schema["type"].as_str(), Some("object"));
    assert_eq!(schema["additionalProperties"].as_bool(), Some(true));
}

#[test]
fn test_plugin_command_included() {
    let mut registry = CommandRegistry::new();
    registry.register(make_cmd("native-cmd", "Native command"));
    // Simulate plugin command by registering it (plugin commands go into registry same way)
    registry.register(make_cmd("plugin-cmd", "Plugin command"));

    let tool_registry = McpToolRegistry::from_command_registry(&registry, "myapp");
    let tools = tool_registry.list_tools();
    assert_eq!(tools.len(), 2);
    let names: Vec<&str> = tools.iter().map(|t| t.name.as_str()).collect();
    assert!(names.contains(&"myapp.native-cmd"));
    assert!(names.contains(&"myapp.plugin-cmd"));
}

#[test]
fn test_hierarchical_command_name_uses_dots() {
    let mut registry = CommandRegistry::new();
    let path = CommandPath::new(&["cluster", "get"]).unwrap();
    let cmd = make_cmd("get", "Get cluster info");
    registry.register_at(&path, cmd).unwrap();

    let tool_registry = McpToolRegistry::from_command_registry(&registry, "myapp");
    let tools = tool_registry.list_tools();
    assert_eq!(tools.len(), 1);
    assert_eq!(tools[0].name, "myapp.cluster.get");
}

#[test]
fn test_tool_descriptor_snapshot() {
    let spec = CommandSpec {
        args: vec![ArgSpec {
            name: "target",
            kind: ArgKind::Option,
            short: None,
            long: None,
            value_type: ArgValueType::String,
            cardinality: Cardinality::Required,
            default: None,
            conflicts_with: vec![],
            requires: vec![],
            help: "Deployment target",
        }],
        ..Default::default()
    };
    let mut registry = CommandRegistry::new();
    registry.register(make_cmd_with_spec("deploy", "Deploy the application", spec));

    let tool_registry = McpToolRegistry::from_command_registry(&registry, "myapp");
    let tools = tool_registry.list_tools();
    assert_eq!(tools.len(), 1);

    let tool = &tools[0];
    assert_eq!(tool.name, "myapp.deploy");
    assert_eq!(tool.description, "Deploy the application");

    // Snapshot test: stable serialized output committed as fixture (AC-SNAPSHOT)
    let mut settings = insta::Settings::clone_current();
    settings.set_snapshot_path(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/snapshots"),
    );
    settings.set_prepend_module_to_snapshot(false);
    settings.bind(|| {
        insta::assert_json_snapshot!("tool_descriptor", tool);
    });
}

#[test]
fn test_expose_mcp_only_filters_commands() {
    let mut registry = CommandRegistry::new();
    registry.register(Command {
        id: "included",
        summary: "Included command",
        syntax: None,
        category: None,
        spec: None,
        validator: None,
        expose_mcp: true,
        execute: noop_execute(),
    });
    registry.register(Command {
        id: "excluded",
        summary: "Excluded command",
        syntax: None,
        category: None,
        spec: None,
        validator: None,
        expose_mcp: false,
        execute: noop_execute(),
    });

    let tool_registry = McpToolRegistry::from_command_registry_with_policy(
        &registry,
        "myapp",
        McpToolExportPolicy::ExposeMcpOnly,
    );
    let tools = tool_registry.list_tools();
    assert_eq!(tools.len(), 1);
    assert_eq!(tools[0].name, "myapp.included");
}

#[test]
fn test_expose_mcp_only_empty_set() {
    let mut registry = CommandRegistry::new();
    registry.register(make_cmd("cmd-a", "Command A"));
    registry.register(make_cmd("cmd-b", "Command B"));

    let tool_registry = McpToolRegistry::from_command_registry_with_policy(
        &registry,
        "myapp",
        McpToolExportPolicy::ExposeMcpOnly,
    );
    assert_eq!(tool_registry.tool_count(), 0);
}

#[tokio::test]
async fn test_call_excluded_tool_returns_not_found() {
    use cli_framework::mcp::dispatch_tool_call;

    let mut registry = CommandRegistry::new();
    registry.register(make_cmd("excluded", "An excluded command"));

    let tool_registry = McpToolRegistry::from_command_registry_with_policy(
        &registry,
        "myapp",
        McpToolExportPolicy::ExposeMcpOnly,
    );

    let result = dispatch_tool_call(
        &tool_registry,
        "myapp.excluded",
        None,
        cli_framework::mcp::McpTransportKind::Http,
    )
    .await;
    assert!(result.is_err());
    let err = result.unwrap_err();
    assert!(
        err.message.starts_with("MCP_CMD_NOT_FOUND:"),
        "expected MCP_CMD_NOT_FOUND error, got: {}",
        err.message
    );
}

#[test]
fn test_all_commands_policy_ignores_flag() {
    let mut registry = CommandRegistry::new();
    registry.register(make_cmd("cmd-a", "Command A"));
    registry.register(make_cmd("cmd-b", "Command B"));

    let tool_registry = McpToolRegistry::from_command_registry_with_policy(
        &registry,
        "myapp",
        McpToolExportPolicy::AllCommands,
    );
    assert_eq!(tool_registry.list_tools().len(), 2);
}
