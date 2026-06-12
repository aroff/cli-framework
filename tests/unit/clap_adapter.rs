use cli_framework::app::AppMeta;
use cli_framework::app::{AppBuilder, Shell};
use cli_framework::command::{Command, CommandRegistry};
use cli_framework::parser::ParseOutcome;
use cli_framework::spec::arg_spec::{ArgKind, ArgSpec, ArgValueType, Cardinality};
use cli_framework::spec::command_tree::CommandSpec;
use cli_framework::spec::value::ArgValue;
use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

fn name_arg() -> ArgSpec {
    ArgSpec {
        name: "name",
        kind: ArgKind::Option,
        value_type: ArgValueType::String,
        cardinality: Cardinality::Optional,
        ..Default::default()
    }
}

fn positional_arg() -> ArgSpec {
    ArgSpec {
        name: "positional",
        kind: ArgKind::Positional,
        value_type: ArgValueType::String,
        cardinality: Cardinality::Optional,
        ..Default::default()
    }
}

fn hello_spec_with_name() -> CommandSpec {
    CommandSpec {
        summary: "Say hello",
        args: vec![name_arg()],
        ..Default::default()
    }
}

fn assert_str_arg(args: &HashMap<String, ArgValue>, key: &str, expected: &str) {
    match args.get(key) {
        Some(ArgValue::Str(s)) => assert_eq!(s, expected, "arg {} mismatch", key),
        other => panic!(
            "expected Str({:?}) for arg {}, got {:?}",
            expected, key, other
        ),
    }
}

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

fn make_registry_with(commands: Vec<Command>) -> CommandRegistry {
    let mut reg = CommandRegistry::new();
    for cmd in commands {
        reg.register(cmd);
    }
    reg
}

struct DummyCtx;
impl cli_framework::app::AppContext for DummyCtx {}

fn make_cmd_with_hidden(id: &'static str, hidden: bool) -> Command {
    Command {
        id: Arc::from(id),
        spec: Arc::new(CommandSpec {
            summary: id,
            hidden,
            args: vec![],
            ..Default::default()
        }),
        validator: None,
        expose_mcp: false,
        expose_chat: true,
        ui: None,
        visibility: None,
        execute: noop_execute(),
    }
}

fn first_non_blank_line(s: &str) -> &str {
    s.lines().find(|l| !l.trim().is_empty()).unwrap_or("")
}

#[test]
fn build_clap_root_subcommand_count_matches_registry() {
    let registry = make_registry_with(vec![
        Command {
            id: Arc::from("hello"),
            spec: Arc::new(CommandSpec {
                summary: "Say hello",
                ..Default::default()
            }),
            validator: None,
            expose_mcp: false,
            expose_chat: true,
            ui: None,
            visibility: None,
            execute: noop_execute(),
        },
        Command {
            id: Arc::from("goodbye"),
            spec: Arc::new(CommandSpec {
                summary: "Say goodbye",
                ..Default::default()
            }),
            validator: None,
            expose_mcp: false,
            expose_chat: true,
            ui: None,
            visibility: None,
            execute: noop_execute(),
        },
    ]);

    let root = cli_framework::app::clap_adapter::build_clap_root(
        None,
        &registry,
        "testapp",
        "0.1.0",
        None,
        &[],
    );

    let matches = root
        .clone()
        .try_get_matches_from(["testapp", "--help"])
        .unwrap_err();
    let help_output = matches.to_string();
    assert!(help_output.contains("hello"));
    assert!(help_output.contains("goodbye"));
}

#[test]
fn build_clap_root_version_propagation() {
    let root = cli_framework::app::clap_adapter::build_clap_root(
        None,
        &CommandRegistry::new(),
        "myapp",
        "2.0.0",
        None,
        &[],
    );

    let err = root
        .clone()
        .try_get_matches_from(["myapp", "--version"])
        .unwrap_err();
    let output = err.to_string();
    assert!(output.contains("myapp"));
    assert!(output.contains("2.0.0"));
}

#[test]
fn build_clap_root_override_usage_from_meta() {
    let meta = AppMeta {
        name: "myapp",
        version: "1.0.0",
        description: "A test app",
        usage: Some("myapp custom-usage <cmd>"),
    };

    let root = cli_framework::app::clap_adapter::build_clap_root(
        Some(&meta),
        &CommandRegistry::new(),
        "myapp",
        "1.0.0",
        None,
        &[],
    );

    let err = root
        .clone()
        .try_get_matches_from(["myapp", "--help"])
        .unwrap_err();
    let help_output = err.to_string();
    assert!(help_output.contains("myapp custom-usage <cmd>"));
}

#[test]
fn build_clap_root_description_from_meta() {
    let meta = AppMeta {
        name: "myapp",
        version: "1.0.0",
        description: "An amazing application",
        usage: None,
    };

    let root = cli_framework::app::clap_adapter::build_clap_root(
        Some(&meta),
        &CommandRegistry::new(),
        "myapp",
        "1.0.0",
        None,
        &[],
    );

    let err = root
        .clone()
        .try_get_matches_from(["myapp", "--help"])
        .unwrap_err();
    let help_output = err.to_string();
    assert!(help_output.contains("An amazing application"));
}

#[test]
fn build_clap_root_adds_version_subcommand_when_not_registered() {
    let root = cli_framework::app::clap_adapter::build_clap_root(
        None,
        &CommandRegistry::new(),
        "testapp",
        "0.1.0",
        None,
        &[],
    );

    let matches = root
        .clone()
        .try_get_matches_from(["testapp", "version"])
        .unwrap();
    let (name, _) = matches.subcommand().unwrap();
    assert_eq!(name, "version");
}

#[test]
fn build_clap_root_no_version_subcommand_when_user_registers_version() {
    let registry = make_registry_with(vec![Command {
        id: Arc::from("version"),
        spec: Arc::new(CommandSpec {
            summary: "Custom version",
            ..Default::default()
        }),
        validator: None,
        expose_mcp: false,
        expose_chat: true,
        ui: None,
        visibility: None,
        execute: noop_execute(),
    }]);

    let root = cli_framework::app::clap_adapter::build_clap_root(
        None,
        &registry,
        "testapp",
        "0.1.0",
        None,
        &[],
    );

    let matches = root
        .clone()
        .try_get_matches_from(["testapp", "version"])
        .unwrap();
    let (name, _sub_matches) = matches.subcommand().unwrap();
    assert_eq!(name, "version");
    // With strict-by-default design (#89), typed commands never use trailing_var_arg.
}

#[test]

fn parse_with_clap_key_value() {
    let registry = make_registry_with(vec![Command {
        id: Arc::from("hello"),
        spec: Arc::new(hello_spec_with_name()),
        validator: None,
        expose_mcp: false,
        expose_chat: true,
        ui: None,
        visibility: None,
        execute: noop_execute(),
    }]);

    let root = cli_framework::app::clap_adapter::build_clap_root(
        None,
        &registry,
        "testapp",
        "0.1.0",
        None,
        &[],
    );

    let outcome = cli_framework::app::clap_adapter::parse_with_clap(
        &root,
        &registry,
        vec![
            "testapp".to_string(),
            "hello".to_string(),
            "--name".to_string(),
            "Alice".to_string(),
        ],
        &[],
        true,
    );

    match outcome {
        ParseOutcome::Parsed {
            command_path, args, ..
        } => {
            assert_eq!(command_path.leaf().unwrap(), "hello");
            assert_str_arg(&args, "name", "Alice");
        }
        other => panic!("expected Parsed, got {:?}", other),
    }
}

#[test]

fn parse_with_clap_key_equals_value() {
    let registry = make_registry_with(vec![Command {
        id: Arc::from("hello"),
        spec: Arc::new(hello_spec_with_name()),
        validator: None,
        expose_mcp: false,
        expose_chat: true,
        ui: None,
        visibility: None,
        execute: noop_execute(),
    }]);

    let root = cli_framework::app::clap_adapter::build_clap_root(
        None,
        &registry,
        "testapp",
        "0.1.0",
        None,
        &[],
    );

    let outcome = cli_framework::app::clap_adapter::parse_with_clap(
        &root,
        &registry,
        vec![
            "testapp".to_string(),
            "hello".to_string(),
            "--name=Alice".to_string(),
        ],
        &[],
        true,
    );

    match outcome {
        ParseOutcome::Parsed {
            command_path, args, ..
        } => {
            assert_eq!(command_path.leaf().unwrap(), "hello");
            assert_str_arg(&args, "name", "Alice");
        }
        other => panic!("expected Parsed, got {:?}", other),
    }
}

#[test]

fn parse_with_clap_positional_after_double_dash() {
    let registry = make_registry_with(vec![Command {
        id: Arc::from("hello"),
        spec: Arc::new(CommandSpec {
            summary: "Say hello",
            args: vec![positional_arg()],
            ..Default::default()
        }),
        validator: None,
        expose_mcp: false,
        expose_chat: true,
        ui: None,
        visibility: None,
        execute: noop_execute(),
    }]);

    let root = cli_framework::app::clap_adapter::build_clap_root(
        None,
        &registry,
        "testapp",
        "0.1.0",
        None,
        &[],
    );

    let outcome = cli_framework::app::clap_adapter::parse_with_clap(
        &root,
        &registry,
        vec![
            "testapp".to_string(),
            "hello".to_string(),
            "--".to_string(),
            "positional".to_string(),
        ],
        &[],
        true,
    );

    match outcome {
        ParseOutcome::Parsed {
            command_path, args, ..
        } => {
            assert_eq!(command_path.leaf().unwrap(), "hello");
            assert!(
                matches!(args.get("positional"), Some(ArgValue::Str(s)) if s == "positional"),
                "expected positional='positional' in args, got: {:?}",
                args
            );
        }
        other => panic!("expected Parsed, got {:?}", other),
    }
}

#[test]

fn parse_with_clap_legacy_leaf_trailing_help_flag_returns_help_shown() {
    let registry = make_registry_with(vec![Command {
        id: Arc::from("hello"),
        spec: Arc::new(CommandSpec {
            summary: "Say hello",
            syntax: Some("testapp hello [-- <args>]"),
            ..Default::default()
        }),
        validator: None,
        expose_mcp: false,
        expose_chat: true,
        ui: None,
        visibility: None,
        execute: noop_execute(),
    }]);

    let root = cli_framework::app::clap_adapter::build_clap_root(
        None,
        &registry,
        "testapp",
        "0.1.0",
        None,
        &[],
    );

    let outcome = cli_framework::app::clap_adapter::parse_with_clap(
        &root,
        &registry,
        vec![
            "testapp".to_string(),
            "hello".to_string(),
            "--help".to_string(),
        ],
        &[],
        true,
    );

    match outcome {
        ParseOutcome::HelpShown(text) => {
            assert!(text.contains("testapp"));
            assert!(text.contains("hello"));
            assert!(text.contains("Say hello"));
            assert!(text.contains("Syntax:"));
        }
        other => panic!("expected HelpShown, got {:?}", other),
    }
}

#[test]

fn parse_with_clap_legacy_leaf_trailing_h_flag_returns_help_shown() {
    let registry = make_registry_with(vec![Command {
        id: Arc::from("hello"),
        spec: Arc::new(CommandSpec {
            summary: "Say hello",
            ..Default::default()
        }),
        validator: None,
        expose_mcp: false,
        expose_chat: true,
        ui: None,
        visibility: None,
        execute: noop_execute(),
    }]);

    let root = cli_framework::app::clap_adapter::build_clap_root(
        None,
        &registry,
        "testapp",
        "0.1.0",
        None,
        &[],
    );

    let outcome = cli_framework::app::clap_adapter::parse_with_clap(
        &root,
        &registry,
        vec!["testapp".to_string(), "hello".to_string(), "-h".to_string()],
        &[],
        true,
    );

    assert!(
        matches!(outcome, ParseOutcome::HelpShown(_)),
        "expected HelpShown, got {:?}",
        outcome
    );
}

#[test]
fn parse_with_clap_legacy_leaf_help_after_terminator_strict_rejects() {
    // With strict-by-default design (#89), commands with no declared positional args
    // reject any trailing args (including -- --help) with an error.
    // This is the intended behavior: args after -- must be declared in the spec.
    let registry = make_registry_with(vec![Command {
        id: Arc::from("hello"),
        spec: Arc::new(CommandSpec {
            summary: "Say hello",
            ..Default::default()
        }),
        validator: None,
        expose_mcp: false,
        expose_chat: true,
        ui: None,
        visibility: None,
        execute: noop_execute(),
    }]);

    let root = cli_framework::app::clap_adapter::build_clap_root(
        None,
        &registry,
        "testapp",
        "0.1.0",
        None,
        &[],
    );

    let outcome = cli_framework::app::clap_adapter::parse_with_clap(
        &root,
        &registry,
        vec![
            "testapp".to_string(),
            "hello".to_string(),
            "--".to_string(),
            "--help".to_string(),
        ],
        &[],
        true,
    );

    // Strict mode: unknown positional args are rejected
    assert!(
        matches!(outcome, ParseOutcome::ParseError(_)),
        "expected ParseError in strict mode, got {:?}",
        outcome
    );
}

#[test]
fn parse_with_clap_help_returns_help_shown() {
    let registry = CommandRegistry::new();
    let root = cli_framework::app::clap_adapter::build_clap_root(
        None,
        &registry,
        "testapp",
        "0.1.0",
        None,
        &[],
    );

    let outcome = cli_framework::app::clap_adapter::parse_with_clap(
        &root,
        &registry,
        vec!["testapp".to_string(), "--help".to_string()],
        &[],
        true,
    );

    assert!(
        matches!(outcome, ParseOutcome::HelpShown(_)),
        "expected HelpShown, got {:?}",
        outcome
    );
}

#[test]
fn parse_with_clap_version_flag_returns_version_shown() {
    let registry = CommandRegistry::new();
    let root = cli_framework::app::clap_adapter::build_clap_root(
        None,
        &registry,
        "testapp",
        "0.1.0",
        None,
        &[],
    );

    let outcome = cli_framework::app::clap_adapter::parse_with_clap(
        &root,
        &registry,
        vec!["testapp".to_string(), "--version".to_string()],
        &[],
        true,
    );

    assert!(
        matches!(outcome, ParseOutcome::VersionShown(_)),
        "expected VersionShown, got {:?}",
        outcome
    );
}

#[test]
fn parse_with_clap_unknown_command_returns_parse_error() {
    let registry = CommandRegistry::new();
    let root = cli_framework::app::clap_adapter::build_clap_root(
        None,
        &registry,
        "testapp",
        "0.1.0",
        None,
        &[],
    );

    let outcome = cli_framework::app::clap_adapter::parse_with_clap(
        &root,
        &registry,
        vec!["testapp".to_string(), "nonexistent".to_string()],
        &[],
        true,
    );

    assert!(
        matches!(outcome, ParseOutcome::ParseError(_)),
        "expected ParseError, got {:?}",
        outcome
    );
}

#[test]
fn parse_with_clap_version_subcommand_returns_parsed() {
    let registry = CommandRegistry::new();
    let root = cli_framework::app::clap_adapter::build_clap_root(
        None,
        &registry,
        "testapp",
        "0.1.0",
        None,
        &[],
    );

    let outcome = cli_framework::app::clap_adapter::parse_with_clap(
        &root,
        &registry,
        vec!["testapp".to_string(), "version".to_string()],
        &[],
        true,
    );

    match outcome {
        ParseOutcome::Parsed { command_path, .. } => {
            assert_eq!(command_path.leaf().unwrap(), "version");
        }
        other => panic!("expected Parsed, got {:?}", other),
    }
}

#[test]
fn emit_completion_bash_stub_shape_and_hidden_filtering() {
    let app = AppBuilder::new()
        .with_version("myapp", "1.0.0")
        .register_command(make_cmd_with_hidden("alpha", false))
        .unwrap()
        .register_command(make_cmd_with_hidden("hidden_cmd", true))
        .unwrap()
        .build(DummyCtx)
        .unwrap();

    let mut out = Vec::<u8>::new();
    app.emit_completion(Shell::Bash, &mut out).unwrap();
    let out = String::from_utf8(out).unwrap();

    let first = first_non_blank_line(&out);
    assert!(
        first.starts_with("_myapp()") || first.starts_with("complete "),
        "unexpected first non-blank line: {:?}",
        first
    );
    assert!(!out.contains("hidden_cmd"));
    assert!(out.contains("compgen -W \""));
}

#[test]
fn emit_completion_zsh_stub_starts_with_compdef() {
    let app = AppBuilder::new()
        .with_version("myapp", "1.0.0")
        .build(DummyCtx)
        .unwrap();

    let mut out = Vec::<u8>::new();
    app.emit_completion(Shell::Zsh, &mut out).unwrap();
    let out = String::from_utf8(out).unwrap();
    assert_eq!(first_non_blank_line(&out), "#compdef myapp");
}

#[test]
fn emit_completion_fish_includes_one_line_per_candidate() {
    let app = AppBuilder::new()
        .with_version("myapp", "1.0.0")
        .register_command(make_cmd_with_hidden("extra", false))
        .unwrap()
        .build(DummyCtx)
        .unwrap();

    let mut out = Vec::<u8>::new();
    app.emit_completion(Shell::Fish, &mut out).unwrap();
    let out = String::from_utf8(out).unwrap();

    assert!(out.contains("complete -c myapp -f"));
    for cmd in ["completion", "extra", "spec"] {
        assert!(
            out.contains(&format!(
                "complete -c myapp -n '__fish_use_subcommand' -a '{}'",
                cmd
            )),
            "missing fish completion line for {}:\n{}",
            cmd,
            out
        );
    }
}

#[test]
fn emit_completion_powershell_contains_register_argument_completer() {
    let app = AppBuilder::new()
        .with_version("myapp", "1.0.0")
        .build(DummyCtx)
        .unwrap();

    let mut out = Vec::<u8>::new();
    app.emit_completion(Shell::PowerShell, &mut out).unwrap();
    let out = String::from_utf8(out).unwrap();
    assert!(out.contains("Register-ArgumentCompleter -Native -CommandName myapp"));
}

#[test]

fn parse_with_clap_mixed_positional_and_named() {
    let registry = make_registry_with(vec![Command {
        id: Arc::from("hello"),
        spec: Arc::new(CommandSpec {
            summary: "Say hello",
            args: vec![positional_arg(), name_arg()],
            ..Default::default()
        }),
        validator: None,
        expose_mcp: false,
        expose_chat: true,
        ui: None,
        visibility: None,
        execute: noop_execute(),
    }]);

    let root = cli_framework::app::clap_adapter::build_clap_root(
        None,
        &registry,
        "testapp",
        "0.1.0",
        None,
        &[],
    );

    let outcome = cli_framework::app::clap_adapter::parse_with_clap(
        &root,
        &registry,
        vec![
            "testapp".to_string(),
            "hello".to_string(),
            "file.txt".to_string(),
            "--name".to_string(),
            "Alice".to_string(),
        ],
        &[],
        true,
    );

    match outcome {
        ParseOutcome::Parsed {
            command_path, args, ..
        } => {
            assert_eq!(command_path.leaf().unwrap(), "hello");
            assert_str_arg(&args, "positional", "file.txt");
            assert_str_arg(&args, "name", "Alice");
        }
        other => panic!("expected Parsed, got {:?}", other),
    }
}

// DD#8 / strict-by-default (#89): unknown flags are always rejected (E002).
// A command must declare args in its spec; undeclared flags are rejected.
#[test]
fn parse_with_clap_unknown_flag_is_rejected_strict() {
    let registry = make_registry_with(vec![Command {
        id: Arc::from("hello"),
        spec: Arc::new(CommandSpec {
            summary: "Say hello",
            ..Default::default()
        }),
        validator: None,
        expose_mcp: false,
        expose_chat: true,
        ui: None,
        visibility: None,
        execute: noop_execute(),
    }]);

    let root = cli_framework::app::clap_adapter::build_clap_root(
        None,
        &registry,
        "testapp",
        "0.1.0",
        None,
        &[],
    );

    let outcome = cli_framework::app::clap_adapter::parse_with_clap(
        &root,
        &registry,
        vec![
            "testapp".to_string(),
            "hello".to_string(),
            "--verbose".to_string(),
        ],
        &[],
        true,
    );

    // With strict-by-default design, unknown flags are always rejected.
    assert!(
        matches!(outcome, ParseOutcome::ParseError(_)),
        "expected ParseError for undeclared flag in strict mode, got {:?}",
        outcome
    );
}

#[test]
#[cfg(feature = "mcp-server")]
fn test_mcp_flags_absent_with_feature() {
    let root = cli_framework::app::clap_adapter::build_clap_root(
        None,
        &CommandRegistry::new(),
        "testapp",
        "0.1.0",
        None,
        &[],
    );
    let arg_longs: Vec<&str> = root.get_arguments().filter_map(|a| a.get_long()).collect();
    assert!(
        !arg_longs.contains(&"mcp-serve"),
        "mcp-serve must be absent with mcp-server feature, got: {:?}",
        arg_longs
    );
    assert!(
        !arg_longs.contains(&"mcp-host"),
        "mcp-host must be absent with mcp-server feature, got: {:?}",
        arg_longs
    );
    assert!(
        !arg_longs.contains(&"mcp-port"),
        "mcp-port must be absent with mcp-server feature, got: {:?}",
        arg_longs
    );
    assert!(
        !arg_longs.contains(&"mcp-path"),
        "mcp-path must be absent with mcp-server feature, got: {:?}",
        arg_longs
    );
}

#[test]
#[cfg(not(feature = "mcp-server"))]
fn test_mcp_flags_absent_without_feature() {
    let root = cli_framework::app::clap_adapter::build_clap_root(
        None,
        &CommandRegistry::new(),
        "testapp",
        "0.1.0",
        None,
        &[],
    );
    // Collect long arg names from root-level args
    let arg_longs: Vec<&str> = root.get_arguments().filter_map(|a| a.get_long()).collect();
    assert!(
        !arg_longs.contains(&"mcp-serve"),
        "mcp-serve must be absent without mcp-server feature"
    );
    assert!(
        !arg_longs.contains(&"mcp-host"),
        "mcp-host must be absent without mcp-server feature"
    );
    assert!(
        !arg_longs.contains(&"mcp-port"),
        "mcp-port must be absent without mcp-server feature"
    );
    assert!(
        !arg_longs.contains(&"mcp-path"),
        "mcp-path must be absent without mcp-server feature"
    );
}

// Stage 1: Multi-segment CommandPath routing (AC-G1 §4.2)

#[test]
fn parse_nested_argv_yields_multi_segment_path() {
    use cli_framework::command::CommandRegistry;
    use cli_framework::spec::command_tree::{CommandPath, CommandSpec};
    use std::sync::Arc;

    let mut registry = CommandRegistry::new();
    let path = CommandPath::new(&["cluster", "get"]).unwrap();
    registry
        .register_at(
            &path,
            Command {
                id: Arc::from("get"),
                spec: Arc::new(CommandSpec {
                    summary: "Get cluster",
                    ..Default::default()
                }),
                validator: None,
                expose_mcp: false,
                expose_chat: true,
                ui: None,
                visibility: None,
                execute: noop_execute(),
            },
        )
        .unwrap();

    let root = cli_framework::app::clap_adapter::build_clap_root(
        None,
        &registry,
        "testapp",
        "0.1.0",
        None,
        &[],
    );
    let outcome = cli_framework::app::clap_adapter::parse_with_clap(
        &root,
        &registry,
        vec![
            "testapp".to_string(),
            "cluster".to_string(),
            "get".to_string(),
        ],
        &[],
        true,
    );

    match outcome {
        ParseOutcome::Parsed { command_path, .. } => {
            assert_eq!(command_path.0.len(), 2, "expected 2-segment path");
            assert_eq!(command_path.0[0], "cluster");
            assert_eq!(command_path.0[1], "get");
        }
        other => panic!("expected Parsed, got {:?}", other),
    }
}

#[test]
fn parse_deep_nested_argv_uses_registered_path_segments() {
    use cli_framework::command::CommandRegistry;
    use cli_framework::spec::command_tree::{CommandPath, CommandSpec};
    use std::sync::Arc;

    let mut registry = CommandRegistry::new();
    let path = CommandPath::new(&["cluster", "node", "get"]).unwrap();
    registry
        .register_at(
            &path,
            Command {
                id: Arc::from("lookup"),
                spec: Arc::new(CommandSpec {
                    summary: "Get cluster node",
                    ..Default::default()
                }),
                validator: None,
                expose_mcp: false,
                expose_chat: true,
                ui: None,
                visibility: None,
                execute: noop_execute(),
            },
        )
        .unwrap();

    let root = cli_framework::app::clap_adapter::build_clap_root(
        None,
        &registry,
        "testapp",
        "0.1.0",
        None,
        &[],
    );
    let outcome = cli_framework::app::clap_adapter::parse_with_clap(
        &root,
        &registry,
        vec![
            "testapp".to_string(),
            "cluster".to_string(),
            "node".to_string(),
            "get".to_string(),
        ],
        &[],
        true,
    );

    match outcome {
        ParseOutcome::Parsed { command_path, .. } => {
            assert_eq!(command_path.0, vec!["cluster", "node", "get"]);
        }
        other => panic!("expected Parsed, got {:?}", other),
    }
}

#[test]
fn parse_nested_group_help_returns_help_shown() {
    use cli_framework::command::CommandRegistry;
    use cli_framework::spec::command_tree::{CommandPath, CommandSpec};
    use std::sync::Arc;

    let mut registry = CommandRegistry::new();
    let path = CommandPath::new(&["cluster", "get"]).unwrap();
    registry
        .register_at(
            &path,
            Command {
                id: Arc::from("get"),
                spec: Arc::new(CommandSpec {
                    summary: "Get cluster",
                    ..Default::default()
                }),
                validator: None,
                expose_mcp: false,
                expose_chat: true,
                ui: None,
                visibility: None,
                execute: noop_execute(),
            },
        )
        .unwrap();

    let root = cli_framework::app::clap_adapter::build_clap_root(
        None,
        &registry,
        "testapp",
        "0.1.0",
        None,
        &[],
    );
    let outcome = cli_framework::app::clap_adapter::parse_with_clap(
        &root,
        &registry,
        vec![
            "testapp".to_string(),
            "cluster".to_string(),
            "--help".to_string(),
        ],
        &[],
        true,
    );

    assert!(
        matches!(outcome, ParseOutcome::HelpShown(_)),
        "expected HelpShown for group --help, got {:?}",
        outcome
    );
}

#[test]
fn parse_unknown_nested_subcommand_returns_e012() {
    use cli_framework::command::CommandRegistry;
    use cli_framework::spec::command_tree::{CommandPath, CommandSpec};
    use std::sync::Arc;

    let mut registry = CommandRegistry::new();
    // Register cluster/get so cluster group exists
    let path = CommandPath::new(&["cluster", "get"]).unwrap();
    registry
        .register_at(
            &path,
            Command {
                id: Arc::from("get"),
                spec: Arc::new(CommandSpec {
                    summary: "Get cluster",
                    ..Default::default()
                }),
                validator: None,
                expose_mcp: false,
                expose_chat: true,
                ui: None,
                visibility: None,
                execute: noop_execute(),
            },
        )
        .unwrap();

    let root = cli_framework::app::clap_adapter::build_clap_root(
        None,
        &registry,
        "testapp",
        "0.1.0",
        None,
        &[],
    );
    // Invoke a non-existent nested command
    let outcome = cli_framework::app::clap_adapter::parse_with_clap(
        &root,
        &registry,
        vec![
            "testapp".to_string(),
            "cluster".to_string(),
            "nonexistent".to_string(),
        ],
        &[],
        true,
    );

    match &outcome {
        ParseOutcome::ParseError(d) => {
            assert_eq!(
                d.code,
                cli_framework::parser::error_codes::E_NESTED_COMMAND_NOT_FOUND,
                "expected E012, got code: {}",
                d.code
            );
        }
        other => panic!("expected ParseError(E012), got {:?}", other),
    }
}

// Verify that --key value and --key=value produce identical CommandArgs (AC-G1.2).
#[test]

fn parse_with_clap_key_value_and_equals_value_parity() {
    let registry = make_registry_with(vec![Command {
        id: Arc::from("hello"),
        spec: Arc::new(hello_spec_with_name()),
        validator: None,
        expose_mcp: false,
        expose_chat: true,
        ui: None,
        visibility: None,
        execute: noop_execute(),
    }]);

    let root = cli_framework::app::clap_adapter::build_clap_root(
        None,
        &registry,
        "testapp",
        "0.1.0",
        None,
        &[],
    );

    let outcome_space = cli_framework::app::clap_adapter::parse_with_clap(
        &root,
        &registry,
        vec![
            "testapp".to_string(),
            "hello".to_string(),
            "--name".to_string(),
            "Alice".to_string(),
        ],
        &[],
        true,
    );

    let outcome_eq = cli_framework::app::clap_adapter::parse_with_clap(
        &root,
        &registry,
        vec![
            "testapp".to_string(),
            "hello".to_string(),
            "--name=Alice".to_string(),
        ],
        &[],
        true,
    );

    match (outcome_space, outcome_eq) {
        (
            ParseOutcome::Parsed {
                command_path: p1,
                args: a1,
                ..
            },
            ParseOutcome::Parsed {
                command_path: p2,
                args: a2,
                ..
            },
        ) => {
            assert_eq!(p1.leaf().unwrap(), p2.leaf().unwrap());
            assert_eq!(a1, a2);
        }
        other => panic!("expected both Parsed, got {:?}", other),
    }
}

// ── "Did you mean?" suggestion tests (G1–G5, G7) ────────────────────────────

#[test]
fn suggest_corrections_e001_close_match_emits_did_you_mean() {
    let registry = make_registry_with(vec![Command {
        id: Arc::from("check"),
        spec: Arc::new(CommandSpec {
            summary: "Run check",
            ..Default::default()
        }),
        validator: None,
        expose_mcp: false,
        expose_chat: true,
        ui: None,
        visibility: None,
        execute: noop_execute(),
    }]);

    let root = cli_framework::app::clap_adapter::build_clap_root(
        None,
        &registry,
        "testapp",
        "0.1.0",
        None,
        &[],
    );

    // "chek" is close enough to "check" for clap's strsim threshold to fire
    let outcome = cli_framework::app::clap_adapter::parse_with_clap(
        &root,
        &registry,
        vec!["testapp".to_string(), "chek".to_string()],
        &[],
        true,
    );

    match outcome {
        ParseOutcome::ParseError(d) => {
            assert_eq!(d.code, "E001");
            let suggestion = d.suggestion.expect("suggestion should be present");
            assert!(
                suggestion.contains("Did you mean 'check'"),
                "expected 'Did you mean' suggestion, got: {}",
                suggestion
            );
        }
        other => panic!("expected ParseError(E001), got {:?}", other),
    }
}

#[test]
fn suggest_corrections_e001_no_match_falls_back_to_generic() {
    let registry = make_registry_with(vec![Command {
        id: Arc::from("serve"),
        spec: Arc::new(CommandSpec {
            summary: "Start server",
            ..Default::default()
        }),
        validator: None,
        expose_mcp: false,
        expose_chat: true,
        ui: None,
        visibility: None,
        execute: noop_execute(),
    }]);

    let root = cli_framework::app::clap_adapter::build_clap_root(
        None,
        &registry,
        "testapp",
        "0.1.0",
        None,
        &[],
    );

    let outcome = cli_framework::app::clap_adapter::parse_with_clap(
        &root,
        &registry,
        vec!["testapp".to_string(), "xyz".to_string()],
        &[],
        true,
    );

    match outcome {
        ParseOutcome::ParseError(d) => {
            assert_eq!(d.code, "E001");
            let suggestion = d.suggestion.expect("suggestion should be present");
            assert!(
                suggestion.contains("Use --help"),
                "expected generic fallback hint, got: {}",
                suggestion
            );
        }
        other => panic!("expected ParseError(E001), got {:?}", other),
    }
}

#[test]
fn suggest_corrections_e002_close_match_emits_did_you_mean() {
    let registry = make_registry_with(vec![Command {
        id: Arc::from("cmd"),
        spec: Arc::new(CommandSpec {
            summary: "A command",
            args: vec![ArgSpec {
                name: "remote-control",
                kind: ArgKind::Option,
                value_type: ArgValueType::String,
                cardinality: Cardinality::Optional,
                ..Default::default()
            }],
            ..Default::default()
        }),
        validator: None,
        expose_mcp: false,
        expose_chat: true,
        ui: None,
        visibility: None,
        execute: noop_execute(),
    }]);

    let root = cli_framework::app::clap_adapter::build_clap_root(
        None,
        &registry,
        "testapp",
        "0.1.0",
        None,
        &[],
    );

    let outcome = cli_framework::app::clap_adapter::parse_with_clap(
        &root,
        &registry,
        vec![
            "testapp".to_string(),
            "cmd".to_string(),
            "--remotecontrol".to_string(),
        ],
        &[],
        true,
    );

    match outcome {
        ParseOutcome::ParseError(d) => {
            assert_eq!(d.code, "E002");
            let suggestion = d.suggestion.expect("suggestion should be present");
            assert!(
                suggestion.contains("Did you mean '--remote-control'"),
                "expected 'Did you mean' suggestion for flag, got: {}",
                suggestion
            );
            assert!(
                d.span.is_some(),
                "span should be populated for E002 with close match"
            );
        }
        other => panic!("expected ParseError(E002), got {:?}", other),
    }
}

#[test]
fn suggest_corrections_e002_no_match_falls_back_to_generic() {
    let registry = make_registry_with(vec![Command {
        id: Arc::from("cmd"),
        spec: Arc::new(CommandSpec {
            summary: "A command",
            args: vec![ArgSpec {
                name: "port",
                kind: ArgKind::Option,
                value_type: ArgValueType::String,
                cardinality: Cardinality::Optional,
                ..Default::default()
            }],
            ..Default::default()
        }),
        validator: None,
        expose_mcp: false,
        expose_chat: true,
        ui: None,
        visibility: None,
        execute: noop_execute(),
    }]);

    let root = cli_framework::app::clap_adapter::build_clap_root(
        None,
        &registry,
        "testapp",
        "0.1.0",
        None,
        &[],
    );

    let outcome = cli_framework::app::clap_adapter::parse_with_clap(
        &root,
        &registry,
        vec![
            "testapp".to_string(),
            "cmd".to_string(),
            "--xyz-totally-unknown".to_string(),
        ],
        &[],
        true,
    );

    match outcome {
        ParseOutcome::ParseError(d) => {
            assert_eq!(d.code, "E002");
            let suggestion = d.suggestion.expect("suggestion should be present");
            assert!(
                suggestion.contains("Use --help"),
                "expected generic fallback hint, got: {}",
                suggestion
            );
        }
        other => panic!("expected ParseError(E002), got {:?}", other),
    }
}

#[test]
fn suggest_corrections_disabled_emits_generic_hint() {
    use cli_framework::app::AppBuilder;

    struct DummyContext;
    impl cli_framework::app::AppContext for DummyContext {}

    let mut app = AppBuilder::new()
        .with_version("testapp", "0.1.0")
        .suggest_corrections(false)
        .register_command(Command {
            id: Arc::from("check"),
            spec: Arc::new(CommandSpec {
                summary: "Run check",
                ..Default::default()
            }),
            validator: None,
            expose_mcp: false,
            expose_chat: true,
            ui: None,
            visibility: None,
            execute: noop_execute(),
        })
        .unwrap()
        .build(DummyContext)
        .unwrap();

    let result = tokio::runtime::Runtime::new()
        .unwrap()
        .block_on(app.run_with_args(vec!["testapp".to_string(), "chek".to_string()]));

    // run_with_args returns Err(UsageError) for parse errors; the diagnostic was already printed.
    // We just verify the call completed (the suggestion check is on DiagnosticReporter output,
    // which this test verifies indirectly via the parse path with suggest_corrections=false).
    assert!(result.is_err(), "expected UsageError for unknown command");

    // Verify via parse_with_clap directly with suggest_corrections=false
    let registry = make_registry_with(vec![Command {
        id: Arc::from("check"),
        spec: Arc::new(CommandSpec {
            summary: "Run check",
            ..Default::default()
        }),
        validator: None,
        expose_mcp: false,
        expose_chat: true,
        ui: None,
        visibility: None,
        execute: noop_execute(),
    }]);

    let root = cli_framework::app::clap_adapter::build_clap_root(
        None,
        &registry,
        "testapp",
        "0.1.0",
        None,
        &[],
    );

    let outcome = cli_framework::app::clap_adapter::parse_with_clap(
        &root,
        &registry,
        vec!["testapp".to_string(), "chek".to_string()],
        &[],
        false,
    );

    match outcome {
        ParseOutcome::ParseError(d) => {
            assert_eq!(d.code, "E001");
            let suggestion = d.suggestion.expect("suggestion should be present");
            assert!(
                suggestion.contains("Use --help"),
                "with suggest_corrections=false, expected generic hint, got: {}",
                suggestion
            );
            assert!(
                !suggestion.contains("Did you mean"),
                "with suggest_corrections=false, should NOT contain 'Did you mean', got: {}",
                suggestion
            );
        }
        other => panic!("expected ParseError(E001), got {:?}", other),
    }
}
