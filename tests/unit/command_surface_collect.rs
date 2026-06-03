use cli_framework::command::{Command, CommandRegistry};
use cli_framework::command_surface::collect::collect;
use cli_framework::spec::command_tree::{CommandPath, CommandSpec};
use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

fn noop_execute() -> Arc<
    dyn for<'a> Fn(
            &'a mut dyn cli_framework::app::AppContext,
            HashMap<String, cli_framework::spec::value::ArgValue>,
        ) -> Pin<Box<dyn Future<Output = anyhow::Result<()>> + Send + 'a>>
        + Send
        + Sync,
> {
    Arc::new(|_ctx, _args| Box::pin(async { Ok(()) }))
}

fn make_cmd(id: &'static str, summary: &'static str) -> Command {
    Command {
        id: Arc::from(id),
        spec: Arc::new(CommandSpec {
            summary,
            ..Default::default()
        }),
        validator: None,
        expose_mcp: false,
        execute: noop_execute(),
    }
}

fn make_cmd_with_spec(id: &'static str, summary: &'static str, spec: CommandSpec) -> Command {
    Command {
        id: Arc::from(id),
        spec: Arc::new(CommandSpec { summary, ..spec }),
        validator: None,
        expose_mcp: false,
        execute: noop_execute(),
    }
}

#[test]
fn empty_registry_returns_empty_commands() {
    let registry = CommandRegistry::new();
    let doc = collect(&registry, "myapp", "1.0.0", false);
    assert!(doc.commands.is_empty());
    assert_eq!(doc.schema_version, "cli-framework.command-surface.v1");
    assert_eq!(doc.app.name, "myapp");
    assert_eq!(doc.app.version, "1.0.0");
}

#[test]
fn root_command_collected() {
    let mut registry = CommandRegistry::new();
    registry.register(make_cmd("deploy", "Deploy the app"));
    let doc = collect(&registry, "myapp", "1.0.0", false);
    assert_eq!(doc.commands.len(), 1);
    assert_eq!(doc.commands[0].path, "deploy");
    assert_eq!(doc.commands[0].id, "deploy");
    assert_eq!(doc.commands[0].summary, "Deploy the app");
}

#[test]
fn hierarchical_command_path_collected() {
    let mut registry = CommandRegistry::new();
    let path = CommandPath::new(&["cluster", "get"]).unwrap();
    registry
        .register_at(&path, make_cmd("get", "Get cluster info"))
        .unwrap();
    let doc = collect(&registry, "myapp", "1.0.0", false);
    assert_eq!(doc.commands.len(), 1);
    assert_eq!(doc.commands[0].path, "cluster/get");
    assert_eq!(doc.commands[0].id, "get");
}

#[test]
fn hidden_command_excluded_without_flag() {
    let mut registry = CommandRegistry::new();
    let spec = CommandSpec {
        hidden: true,
        ..Default::default()
    };
    registry.register(make_cmd_with_spec("internal", "Internal command", spec));
    let doc = collect(&registry, "myapp", "1.0.0", false);
    assert!(doc.commands.is_empty(), "hidden command should be excluded");
}

#[test]
fn hidden_command_included_with_flag() {
    let mut registry = CommandRegistry::new();
    let spec = CommandSpec {
        hidden: true,
        ..Default::default()
    };
    registry.register(make_cmd_with_spec("internal", "Internal command", spec));
    let doc = collect(&registry, "myapp", "1.0.0", true);
    assert_eq!(doc.commands.len(), 1);
    assert_eq!(doc.commands[0].hidden, true);
}

#[test]
fn empty_spec_command_collected() {
    // With spec #89, all commands must have a spec (mandatory).
    // A command with an empty spec (no args declared) produces a typed empty schema.
    let mut registry = CommandRegistry::new();
    registry.register(make_cmd("hello", "Say hello"));
    let doc = collect(&registry, "myapp", "1.0.0", false);
    assert_eq!(doc.commands.len(), 1);
    // typed empty spec: schema is {"type": "object", "properties": {}}
    let schema = &doc.commands[0].input_schema;
    assert_eq!(schema["type"].as_str(), Some("object"));
    assert!(schema["properties"].is_object());
    assert_eq!(doc.commands[0].hidden, false);
    assert!(doc.commands[0].args.is_empty());
}

#[test]
fn commands_sorted_by_path() {
    let mut registry = CommandRegistry::new();
    registry.register(make_cmd("zebra", "Z"));
    registry.register(make_cmd("alpha", "A"));
    registry.register(make_cmd("middle", "M"));
    let doc = collect(&registry, "myapp", "1.0.0", false);
    let paths: Vec<&str> = doc.commands.iter().map(|c| c.path.as_str()).collect();
    assert_eq!(paths, vec!["alpha", "middle", "zebra"]);
}

#[test]
fn spec_command_args_mapped() {
    use cli_framework::spec::arg_spec::{ArgKind, ArgSpec, ArgValueType, Cardinality};
    let mut registry = CommandRegistry::new();
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
            help: "Target environment",
            ..Default::default()
        }],
        ..Default::default()
    };
    registry.register(make_cmd_with_spec("deploy", "Deploy", spec));
    let doc = collect(&registry, "myapp", "1.0.0", false);
    let cmd = &doc.commands[0];
    assert_eq!(cmd.args.len(), 1);
    assert_eq!(cmd.args[0].name, "env");
    assert_eq!(cmd.args[0].kind, "option");
    assert_eq!(cmd.args[0].cardinality, "required");
}
