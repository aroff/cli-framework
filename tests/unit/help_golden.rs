//! Spec #89 framework test layer 4: help golden snapshots.
//! Verifies per-subcommand help renders real args from the typed spec —
//! not the old legacy stub ("Command help (legacy)").
//!
//! ADR 0061/0064/0065: `<cmd> --help` and `<cmd> <sub> --help` render
//! from build_typed_clap_command; no "Command help (legacy)" or "[trailing]" appears.

use cli_framework::app::clap_adapter::{build_clap_root, parse_with_clap};
use cli_framework::command::{Command, CommandRegistry};
use cli_framework::parser::outcome::ParseOutcome;
use cli_framework::spec::arg_spec::{ArgKind, ArgSpec, ArgValueType, Cardinality};
use cli_framework::spec::command_tree::{CommandPath, CommandSpec, GroupMetadata};
use std::sync::Arc;

fn make_typed_cmd(id: &'static str, spec: CommandSpec) -> Command {
    Command {
        id: Arc::from(id),
        spec: Arc::new(spec),
        validator: None,
        expose_mcp: false,
        expose_chat: true,
        ui: None,
        visibility: None,
        execute: Arc::new(|_ctx, _args| Box::pin(async { Ok(()) })),
    }
}

fn make_hello_spec() -> CommandSpec {
    CommandSpec {
        summary: "Say hello to someone",
        long_about: Some("Extended description of the hello command."),
        syntax: Some("hello --name <name> [--count <n>]"),
        args: vec![
            ArgSpec {
                name: "name",
                kind: ArgKind::Option,
                long: Some("name"),
                short: Some('n'),
                value_type: ArgValueType::String,
                cardinality: Cardinality::Required,
                help: "Name to greet",
                ..Default::default()
            },
            ArgSpec {
                name: "count",
                kind: ArgKind::Option,
                long: Some("count"),
                value_type: ArgValueType::Int,
                cardinality: Cardinality::Optional,
                help: "Number of times to greet",
                ..Default::default()
            },
        ],
        ..Default::default()
    }
}

// ── Layer 4a: flat command --help ─────────────────────────────────────────────

#[test]
fn flat_command_help_renders_typed_spec_args() {
    let mut registry = CommandRegistry::new();
    registry.register(make_typed_cmd("hello", make_hello_spec()));

    let root = build_clap_root(None, &registry, "testapp", "0.1.0", None, &[]);
    let outcome = parse_with_clap(
        &root,
        &registry,
        vec!["testapp".into(), "hello".into(), "--help".into()],
        &[],
        true,
    );

    let help_text = match outcome {
        ParseOutcome::HelpShown(text) => text,
        other => panic!("expected HelpShown, got {:?}", other),
    };

    // Must show the command description (long_about takes precedence in --help output)
    assert!(
        help_text.contains("Say hello to someone") || help_text.contains("Extended description"),
        "help must contain command description; got:\n{}",
        help_text
    );
    // Must show the declared args — not "[trailing]"
    assert!(
        help_text.contains("--name"),
        "help must show --name arg; got:\n{}",
        help_text
    );
    assert!(
        help_text.contains("--count"),
        "help must show --count arg; got:\n{}",
        help_text
    );
    // Must NOT contain legacy stub text
    assert!(
        !help_text.contains("trailing"),
        "help must NOT contain 'trailing' (legacy path); got:\n{}",
        help_text
    );
    assert!(
        !help_text.contains("Command help (legacy)"),
        "help must NOT show legacy stub; got:\n{}",
        help_text
    );
    // Syntax hint from spec.syntax via after_help
    assert!(
        help_text.contains("Syntax:"),
        "help must show Syntax: from spec.syntax; got:\n{}",
        help_text
    );
}

#[test]
fn flat_command_help_shows_short_flag() {
    let mut registry = CommandRegistry::new();
    registry.register(make_typed_cmd("hello", make_hello_spec()));

    let root = build_clap_root(None, &registry, "testapp", "0.1.0", None, &[]);
    let outcome = parse_with_clap(
        &root,
        &registry,
        vec!["testapp".into(), "hello".into(), "--help".into()],
        &[],
        true,
    );

    if let ParseOutcome::HelpShown(text) = outcome {
        assert!(
            text.contains("-n") || text.contains("--name"),
            "help should show short flag -n or long --name; got:\n{}",
            text
        );
    } else {
        panic!("expected HelpShown");
    }
}

// ── Layer 4b: nested command help ─────────────────────────────────────────────

#[test]
fn nested_group_help_renders_subcommands() {
    let mut registry = CommandRegistry::new();
    registry
        .register_group(
            &CommandPath::root_for("greet"),
            GroupMetadata {
                summary: "Greeting commands",
                hidden: false,
            },
        )
        .unwrap();
    let hello_path = CommandPath::new(&["greet", "hello"]).unwrap();
    registry
        .register_at(&hello_path, make_typed_cmd("hello", make_hello_spec()))
        .unwrap();

    let root = build_clap_root(None, &registry, "testapp", "0.1.0", None, &[]);
    let outcome = parse_with_clap(
        &root,
        &registry,
        vec!["testapp".into(), "greet".into(), "--help".into()],
        &[],
        true,
    );

    let help_text = match outcome {
        ParseOutcome::HelpShown(text) => text,
        other => panic!("expected HelpShown for group help, got {:?}", other),
    };

    assert!(
        help_text.contains("hello"),
        "group help should list 'hello' subcommand; got:\n{}",
        help_text
    );
    assert!(
        !help_text.contains("Command help (legacy)"),
        "group help must not show legacy stub; got:\n{}",
        help_text
    );
}

#[test]
fn nested_subcommand_help_renders_typed_args() {
    let mut registry = CommandRegistry::new();
    registry
        .register_group(
            &CommandPath::root_for("greet"),
            GroupMetadata {
                summary: "Greeting commands",
                hidden: false,
            },
        )
        .unwrap();
    let hello_path = CommandPath::new(&["greet", "hello"]).unwrap();
    registry
        .register_at(&hello_path, make_typed_cmd("hello", make_hello_spec()))
        .unwrap();

    let root = build_clap_root(None, &registry, "testapp", "0.1.0", None, &[]);
    let outcome = parse_with_clap(
        &root,
        &registry,
        vec![
            "testapp".into(),
            "greet".into(),
            "hello".into(),
            "--help".into(),
        ],
        &[],
        true,
    );

    let help_text = match outcome {
        ParseOutcome::HelpShown(text) => text,
        other => panic!(
            "expected HelpShown for nested subcommand help, got {:?}",
            other
        ),
    };

    // Typed args must appear — not legacy stub
    assert!(
        help_text.contains("--name"),
        "nested subcommand help must show --name arg; got:\n{}",
        help_text
    );
    assert!(
        !help_text.contains("trailing"),
        "nested help must not contain legacy trailing; got:\n{}",
        help_text
    );
}

// ── Layer 4c: Options: block includes global flags ─────────────────────────────

#[test]
fn root_help_options_block_lists_global_flags() {
    use cli_framework::app::{AppBuilder, AppContext};

    // Build an app with one global flag
    struct Ctx;
    impl AppContext for Ctx {}

    let global_verbose = ArgSpec {
        name: "verbose",
        kind: ArgKind::Flag,
        long: Some("verbose"),
        short: Some('v'),
        value_type: ArgValueType::Bool,
        cardinality: Cardinality::Optional,
        help: "Enable verbose output",
        ..Default::default()
    };

    let app = AppBuilder::new()
        .with_version("myapp", "1.0.0")
        .global_flag(global_verbose)
        .without_completion()
        .build(Ctx)
        .unwrap();

    let help = app.render_help();

    // Options: block must list the registered global flag
    assert!(
        help.contains("--verbose") || help.contains("Options:"),
        "help must contain Options: section; got:\n{}",
        help
    );
    assert!(
        help.contains("--verbose"),
        "Options: block must list global --verbose; got:\n{}",
        help
    );
    assert!(
        help.contains("Enable verbose output"),
        "Options: block must include help text for global flag; got:\n{}",
        help
    );
}

// ── Layer 5b: MCP schema golden — tool name format ───────────────────────────

#[test]
fn mcp_tool_name_uses_underscore_path_separator() {
    use cli_framework::command::CommandRegistry;
    use cli_framework::mcp::{McpToolExportPolicy, McpToolRegistry};
    use std::sync::Arc;

    let mut registry = CommandRegistry::new();
    let path = CommandPath::new(&["app", "sub"]).unwrap();
    registry
        .register_at(
            &path,
            make_typed_cmd(
                "sub",
                CommandSpec {
                    summary: "nested sub",
                    ..Default::default()
                },
            ),
        )
        .unwrap();

    let arc_reg = Arc::new(registry);
    let tool_registry = McpToolRegistry::from_command_registry_with_policy(
        &arc_reg,
        "myapp",
        McpToolExportPolicy::AllCommands,
    );

    let tools = tool_registry.list_tools();
    let sub_tool = tools
        .iter()
        .find(|t| t.name.contains("sub"))
        .expect("sub tool");

    // Tool name must use _ as separator (ADR glossary: app_a_b for path [a, b])
    assert!(
        sub_tool.name.contains('_'),
        "MCP tool name for nested command must use _ separator; got: {}",
        sub_tool.name
    );
    assert!(
        !sub_tool.name.contains('/'),
        "MCP tool name must NOT use / separator; got: {}",
        sub_tool.name
    );
}
