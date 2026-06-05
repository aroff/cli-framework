//! Spec #89 framework test layer 1+2+6: derive unit tests, typed-extraction round-trips,
//! infallibility invariant, and CLI↔MCP parity (ADR 0061/0063/0064/0065).

use cli_framework::command::{FromArgValueMap, IntoCommandSpec};
use cli_framework::spec::arg_spec::{ArgKind, ArgValueType, Cardinality};
use cli_framework::spec::value::ArgValue;
use std::collections::HashMap;
use std::path::PathBuf;

// ── Manual implementations used in tests (pre-derive) ─────────────────────────

/// Minimal typed args struct — manually implements the traits that #[derive(CommandSpec)]
/// would generate.  Proves the round-trip and infallibility invariant without
/// requiring the proc-macro to be compiled.
struct RunArgs {
    config: PathBuf,
    out_dir: Option<PathBuf>,
    verbose: bool,
    count: i64,
    tags: Vec<String>,
}

impl IntoCommandSpec for RunArgs {
    fn command_spec() -> cli_framework::spec::command_tree::CommandSpec {
        use cli_framework::spec::arg_spec::ArgSpec;
        use cli_framework::spec::command_tree::CommandSpec;
        CommandSpec {
            summary: "Run skill optimization from a config file",
            category: Some("quality"),
            syntax: Some("run --config <path>"),
            args: vec![
                ArgSpec {
                    name: "config",
                    kind: ArgKind::Option,
                    long: Some("config"),
                    value_type: ArgValueType::String,
                    cardinality: Cardinality::Required,
                    help: "Path to config file",
                    ..Default::default()
                },
                ArgSpec {
                    name: "out-dir",
                    kind: ArgKind::Option,
                    long: Some("out-dir"),
                    value_type: ArgValueType::String,
                    cardinality: Cardinality::Optional,
                    help: "Output directory",
                    ..Default::default()
                },
                ArgSpec {
                    name: "verbose",
                    kind: ArgKind::Flag,
                    long: Some("verbose"),
                    value_type: ArgValueType::Bool,
                    cardinality: Cardinality::Optional,
                    short: Some('v'),
                    help: "Verbose output",
                    ..Default::default()
                },
                ArgSpec {
                    name: "count",
                    kind: ArgKind::Option,
                    long: Some("count"),
                    value_type: ArgValueType::Int,
                    cardinality: Cardinality::Optional,
                    help: "Iteration count",
                    ..Default::default()
                },
                ArgSpec {
                    name: "tag",
                    kind: ArgKind::Option,
                    long: Some("tag"),
                    value_type: ArgValueType::String,
                    cardinality: Cardinality::Repeated,
                    help: "Tag (repeatable)",
                    ..Default::default()
                },
            ],
            ..Default::default()
        }
    }
}

impl FromArgValueMap for RunArgs {
    fn from_arg_value_map(map: &HashMap<String, ArgValue>) -> Self {
        Self {
            config: match map.get("config") {
                Some(ArgValue::Str(s)) => PathBuf::from(s),
                _ => panic!("framework bug: required 'config' missing from validated map"),
            },
            out_dir: map.get("out-dir").and_then(|v| {
                if let ArgValue::Str(s) = v {
                    Some(PathBuf::from(s))
                } else {
                    None
                }
            }),
            verbose: matches!(map.get("verbose"), Some(ArgValue::Bool(true))),
            count: match map.get("count") {
                Some(ArgValue::Int(i)) => *i,
                _ => 0,
            },
            tags: match map.get("tag") {
                Some(ArgValue::List(items)) => items
                    .iter()
                    .filter_map(|v| {
                        if let ArgValue::Str(s) = v {
                            Some(s.clone())
                        } else {
                            None
                        }
                    })
                    .collect(),
                _ => vec![],
            },
        }
    }
}

// ── Layer 1: Derive unit tests — CommandSpec shape ─────────────────────────────

#[test]
fn derive_command_spec_summary_and_category() {
    let spec = RunArgs::command_spec();
    assert_eq!(spec.summary, "Run skill optimization from a config file");
    assert_eq!(spec.category, Some("quality"));
    assert_eq!(spec.syntax, Some("run --config <path>"));
}

#[test]
fn derive_command_spec_arg_count() {
    let spec = RunArgs::command_spec();
    assert_eq!(
        spec.args.len(),
        5,
        "expected 5 args (config, out-dir, verbose, count, tag)"
    );
}

#[test]
fn derive_command_spec_required_arg() {
    let spec = RunArgs::command_spec();
    let config = spec
        .args
        .iter()
        .find(|a| a.name == "config")
        .expect("config arg");
    assert_eq!(config.kind, ArgKind::Option);
    assert_eq!(config.value_type, ArgValueType::String);
    assert_eq!(config.cardinality, Cardinality::Required);
    assert_eq!(config.long, Some("config"));
}

#[test]
fn derive_command_spec_optional_path_arg() {
    let spec = RunArgs::command_spec();
    let out = spec
        .args
        .iter()
        .find(|a| a.name == "out-dir")
        .expect("out-dir arg");
    assert_eq!(out.cardinality, Cardinality::Optional);
    assert_eq!(out.value_type, ArgValueType::String);
}

#[test]
fn derive_command_spec_bool_flag() {
    let spec = RunArgs::command_spec();
    let verbose = spec
        .args
        .iter()
        .find(|a| a.name == "verbose")
        .expect("verbose arg");
    assert_eq!(verbose.kind, ArgKind::Flag);
    assert_eq!(verbose.value_type, ArgValueType::Bool);
    assert_eq!(verbose.short, Some('v'));
}

#[test]
fn derive_command_spec_int_arg() {
    let spec = RunArgs::command_spec();
    let count = spec
        .args
        .iter()
        .find(|a| a.name == "count")
        .expect("count arg");
    assert_eq!(count.value_type, ArgValueType::Int);
    assert_eq!(count.cardinality, Cardinality::Optional);
}

#[test]
fn derive_command_spec_repeated_arg() {
    let spec = RunArgs::command_spec();
    let tag = spec.args.iter().find(|a| a.name == "tag").expect("tag arg");
    assert_eq!(tag.cardinality, Cardinality::Repeated);
    assert_eq!(tag.value_type, ArgValueType::String);
}

// ── Layer 2: Typed-extraction round-trips ─────────────────────────────────────

#[test]
fn extraction_required_string_as_pathbuf() {
    let mut map = HashMap::new();
    map.insert("config".into(), ArgValue::Str("/etc/cfg.toml".into()));
    let args = RunArgs::from_arg_value_map(&map);
    assert_eq!(args.config, PathBuf::from("/etc/cfg.toml"));
}

#[test]
fn extraction_optional_present() {
    let mut map = HashMap::new();
    map.insert("config".into(), ArgValue::Str("a.toml".into()));
    map.insert("out-dir".into(), ArgValue::Str("/tmp/out".into()));
    let args = RunArgs::from_arg_value_map(&map);
    assert_eq!(args.out_dir, Some(PathBuf::from("/tmp/out")));
}

#[test]
fn extraction_optional_absent() {
    let mut map = HashMap::new();
    map.insert("config".into(), ArgValue::Str("a.toml".into()));
    let args = RunArgs::from_arg_value_map(&map);
    assert!(args.out_dir.is_none());
}

#[test]
fn extraction_bool_flag_true() {
    let mut map = HashMap::new();
    map.insert("config".into(), ArgValue::Str("a.toml".into()));
    map.insert("verbose".into(), ArgValue::Bool(true));
    let args = RunArgs::from_arg_value_map(&map);
    assert!(args.verbose);
}

#[test]
fn extraction_bool_flag_absent_is_false() {
    let mut map = HashMap::new();
    map.insert("config".into(), ArgValue::Str("a.toml".into()));
    let args = RunArgs::from_arg_value_map(&map);
    assert!(!args.verbose);
}

#[test]
fn extraction_int_present() {
    let mut map = HashMap::new();
    map.insert("config".into(), ArgValue::Str("a.toml".into()));
    map.insert("count".into(), ArgValue::Int(42));
    let args = RunArgs::from_arg_value_map(&map);
    assert_eq!(args.count, 42);
}

#[test]
fn extraction_int_absent_defaults_to_zero() {
    let mut map = HashMap::new();
    map.insert("config".into(), ArgValue::Str("a.toml".into()));
    let args = RunArgs::from_arg_value_map(&map);
    assert_eq!(args.count, 0);
}

#[test]
fn extraction_repeated_list() {
    let mut map = HashMap::new();
    map.insert("config".into(), ArgValue::Str("a.toml".into()));
    map.insert(
        "tag".into(),
        ArgValue::List(vec![
            ArgValue::Str("alpha".into()),
            ArgValue::Str("beta".into()),
        ]),
    );
    let args = RunArgs::from_arg_value_map(&map);
    assert_eq!(args.tags, vec!["alpha", "beta"]);
}

#[test]
fn extraction_repeated_absent_is_empty() {
    let mut map = HashMap::new();
    map.insert("config".into(), ArgValue::Str("a.toml".into()));
    let args = RunArgs::from_arg_value_map(&map);
    assert!(args.tags.is_empty());
}

// ── Layer 2b: Infallibility invariant (ADR 0063) ──────────────────────────────
// A validated map always projects without error.
// We can't test the panic path in a normal test, but we verify that all
// cardinality / type combinations produce a value without returning Result.

#[test]
fn infallibility_all_argvalue_types_produce_value() {
    use cli_framework::spec::value::ArgValue::*;

    // Simulate a fully-populated validated map for RunArgs
    let mut map = HashMap::new();
    map.insert("config".into(), Str("/cfg".into()));
    map.insert("out-dir".into(), Str("/out".into()));
    map.insert("verbose".into(), Bool(true));
    map.insert("count".into(), Int(7));
    map.insert("tag".into(), List(vec![Str("a".into()), Str("b".into())]));

    // This must not panic — the invariant says validated maps are infallible.
    let args = RunArgs::from_arg_value_map(&map);
    assert_eq!(args.config, PathBuf::from("/cfg"));
    assert_eq!(args.out_dir, Some(PathBuf::from("/out")));
    assert!(args.verbose);
    assert_eq!(args.count, 7);
    assert_eq!(args.tags, vec!["a", "b"]);
}

// ── Layer 6: CLI↔MCP parity (ADR 0061) ────────────────────────────────────────
// The same (command, args) resolves to the same typed T whether it arrives
// as argv or as an MCP JSON tool call (both converge on the ArgValue map).

#[test]
fn cli_mcp_parity_identical_extraction() {
    use cli_framework::mcp::json_value_to_typed_map;

    // Simulate what clap→map_matches_to_typed_args produces for CLI path
    let mut cli_map = HashMap::new();
    cli_map.insert("config".into(), ArgValue::Str("/a/b.toml".into()));
    cli_map.insert("count".into(), ArgValue::Int(3));
    cli_map.insert("verbose".into(), ArgValue::Bool(true));

    // Simulate what an MCP tool call JSON produces
    let json_obj = serde_json::json!({
        "config": "/a/b.toml",
        "count": 3,
        "verbose": true,
    });
    let mcp_map = json_value_to_typed_map(json_obj.as_object().unwrap());

    // Both maps should extract the same RunArgs
    let from_cli = RunArgs::from_arg_value_map(&cli_map);
    let from_mcp = RunArgs::from_arg_value_map(&mcp_map);

    assert_eq!(from_cli.config, from_mcp.config);
    assert_eq!(from_cli.verbose, from_mcp.verbose);
    assert_eq!(from_cli.count, from_mcp.count);
}

// ── Layer 3: Validation — constraint checks (ADR 0063) ────────────────────────

#[test]
fn constraint_int_min_max_in_spec() {
    use cli_framework::spec::arg_spec::ArgSpec;
    use cli_framework::spec::command_tree::CommandSpec;

    let spec = CommandSpec {
        summary: "test",
        args: vec![ArgSpec {
            name: "port",
            kind: ArgKind::Option,
            value_type: ArgValueType::Int,
            cardinality: Cardinality::Optional,
            min: Some(1),
            max: Some(65535),
            help: "Port number",
            ..Default::default()
        }],
        ..Default::default()
    };

    // Below minimum → E004
    let mut args = HashMap::new();
    args.insert("port".into(), ArgValue::Int(0));
    let diags = spec.validate_typed_args(&args);
    assert!(
        diags.iter().any(|d| d.code == "E004"),
        "expected E004 for port=0"
    );

    // Above maximum → E004
    let mut args = HashMap::new();
    args.insert("port".into(), ArgValue::Int(70000));
    let diags = spec.validate_typed_args(&args);
    assert!(
        diags.iter().any(|d| d.code == "E004"),
        "expected E004 for port=70000"
    );

    // In range → no error
    let mut args = HashMap::new();
    args.insert("port".into(), ArgValue::Int(8080));
    let diags = spec.validate_typed_args(&args);
    assert!(diags.is_empty(), "no error for valid port: {:?}", diags);
}

#[test]
fn constraint_string_pattern_in_spec() {
    use cli_framework::spec::arg_spec::ArgSpec;
    use cli_framework::spec::command_tree::CommandSpec;

    let spec = CommandSpec {
        summary: "test",
        args: vec![ArgSpec {
            name: "name",
            kind: ArgKind::Option,
            value_type: ArgValueType::String,
            cardinality: Cardinality::Optional,
            pattern: Some(r"^[a-z][a-z0-9_-]*$"),
            help: "Identifier",
            ..Default::default()
        }],
        ..Default::default()
    };

    // Invalid pattern → E004
    let mut args = HashMap::new();
    args.insert("name".into(), ArgValue::Str("123invalid".into()));
    let diags = spec.validate_typed_args(&args);
    assert!(
        diags.iter().any(|d| d.code == "E004"),
        "expected E004 for invalid pattern"
    );

    // Valid → no error
    let mut args = HashMap::new();
    args.insert("name".into(), ArgValue::Str("my-skill".into()));
    let diags = spec.validate_typed_args(&args);
    assert!(diags.is_empty(), "no error for valid name: {:?}", diags);
}

#[test]
fn constraint_float_range_in_spec() {
    use cli_framework::spec::arg_spec::ArgSpec;
    use cli_framework::spec::command_tree::CommandSpec;

    let spec = CommandSpec {
        summary: "test",
        args: vec![ArgSpec {
            name: "threshold",
            kind: ArgKind::Option,
            value_type: ArgValueType::Float,
            cardinality: Cardinality::Optional,
            min_f: Some(0.0),
            max_f: Some(1.0),
            help: "Confidence threshold 0.0-1.0",
            ..Default::default()
        }],
        ..Default::default()
    };

    let mut args = HashMap::new();
    args.insert("threshold".into(), ArgValue::Float(-0.1));
    let diags = spec.validate_typed_args(&args);
    assert!(
        diags.iter().any(|d| d.code == "E004"),
        "expected E004 for threshold=-0.1"
    );

    let mut args = HashMap::new();
    args.insert("threshold".into(), ArgValue::Float(0.85));
    let diags = spec.validate_typed_args(&args);
    assert!(
        diags.is_empty(),
        "no error for valid threshold: {:?}",
        diags
    );
}

// ── Layer 1b: MCP JSON Schema reflects constraints ─────────────────────────────

#[test]
fn constraint_int_min_max_appear_in_json_schema() {
    use cli_framework::spec::arg_spec::ArgSpec;

    let arg = ArgSpec {
        name: "port",
        kind: ArgKind::Option,
        value_type: ArgValueType::Int,
        cardinality: Cardinality::Optional,
        min: Some(1),
        max: Some(65535),
        help: "",
        ..Default::default()
    };
    let (prop_name, schema) = arg.to_json_schema_property();
    assert_eq!(prop_name, "port");
    assert_eq!(schema["type"].as_str(), Some("integer"));
    assert_eq!(schema["minimum"].as_i64(), Some(1));
    assert_eq!(schema["maximum"].as_i64(), Some(65535));
}

#[test]
fn constraint_string_pattern_appears_in_json_schema() {
    use cli_framework::spec::arg_spec::ArgSpec;

    let arg = ArgSpec {
        name: "name",
        kind: ArgKind::Option,
        value_type: ArgValueType::String,
        cardinality: Cardinality::Optional,
        pattern: Some(r"^[a-z]+$"),
        help: "",
        ..Default::default()
    };
    let (_, schema) = arg.to_json_schema_property();
    assert_eq!(schema["type"].as_str(), Some("string"));
    assert_eq!(schema["pattern"].as_str(), Some(r"^[a-z]+$"));
}

// ── Layer 5: MCP schema: globals absent, constraints present ──────────────────

#[test]
fn mcp_schema_globals_absent() {
    use cli_framework::command::Command;
    use cli_framework::mcp::McpToolRegistry;
    use cli_framework::spec::arg_spec::ArgSpec;
    use cli_framework::spec::command_tree::CommandSpec;
    use std::sync::Arc;

    // Global flag is intentionally not registered here — test verifies
    // that even without a registered global, the tool schema is clean.
    let _global_verbose = ArgSpec {
        name: "verbose",
        kind: ArgKind::Flag,
        value_type: ArgValueType::Bool,
        cardinality: Cardinality::Optional,
        short: Some('v'),
        help: "Verbose output",
        ..Default::default()
    };

    // Command with one local arg
    let spec = CommandSpec {
        summary: "Say hello",
        args: vec![ArgSpec {
            name: "name",
            kind: ArgKind::Option,
            long: Some("name"),
            value_type: ArgValueType::String,
            cardinality: Cardinality::Required,
            help: "Name to greet",
            ..Default::default()
        }],
        ..Default::default()
    };

    let registry = {
        use cli_framework::command::CommandRegistry;
        let mut reg = CommandRegistry::new();
        reg.register(Command {
            id: Arc::from("hello"),
            spec: Arc::new(spec),
            validator: None,
            expose_mcp: true,
            expose_chat: true,
            execute: Arc::new(|_ctx, _args| Box::pin(async { Ok(()) })),
        });
        Arc::new(reg)
    };

    let tool_registry = McpToolRegistry::from_command_registry_with_policy(
        &registry,
        "testapp",
        cli_framework::mcp::McpToolExportPolicy::AllCommands,
    );

    let tools = tool_registry.list_tools();
    let hello_tool = tools
        .iter()
        .find(|t| t.name.contains("hello"))
        .expect("hello tool");

    let schema = &hello_tool.input_schema;

    // Global flag must NOT appear in tool schema (ADR 0062)
    let props = schema["properties"].as_object().expect("properties object");
    assert!(
        !props.contains_key("verbose"),
        "global 'verbose' flag must NOT appear in tool schema, but found it: {:#?}",
        props
    );

    // Local arg must appear
    assert!(
        props.contains_key("name"),
        "'name' arg must appear in tool schema, but missing: {:#?}",
        props
    );
}
