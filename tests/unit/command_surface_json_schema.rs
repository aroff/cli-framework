use cli_framework::command_surface::json_schema::build_input_schema;
use cli_framework::spec::arg_spec::{ArgKind, ArgSpec, ArgValueType, Cardinality};
use cli_framework::spec::command_tree::CommandSpec;

fn make_arg(
    name: &'static str,
    kind: ArgKind,
    value_type: ArgValueType,
    cardinality: Cardinality,
) -> ArgSpec {
    ArgSpec {
        name,
        kind,
        short: None,
        long: None,
        value_type,
        cardinality,
        default: None,
        conflicts_with: vec![],
        requires: vec![],
        help: "test arg",
        ..Default::default()
    }
}

#[test]
fn spec_none_returns_permissive_schema() {
    let schema = build_input_schema(None);
    assert_eq!(schema["type"].as_str(), Some("object"));
    assert_eq!(schema["additionalProperties"].as_bool(), Some(true));
}

#[test]
fn flag_arg_bool_schema() {
    let arg = make_arg(
        "verbose",
        ArgKind::Flag,
        ArgValueType::Bool,
        Cardinality::Optional,
    );
    let (name, schema) = arg.to_json_schema_property();
    assert_eq!(name, "verbose");
    assert_eq!(schema["type"].as_str(), Some("boolean"));
}

#[test]
fn option_string_schema() {
    let arg = make_arg(
        "env",
        ArgKind::Option,
        ArgValueType::String,
        Cardinality::Optional,
    );
    let (name, schema) = arg.to_json_schema_property();
    assert_eq!(name, "env");
    assert_eq!(schema["type"].as_str(), Some("string"));
}

#[test]
fn enum_schema() {
    let arg = make_arg(
        "format",
        ArgKind::Option,
        ArgValueType::Enum(vec!["json", "yaml"]),
        Cardinality::Optional,
    );
    let (name, schema) = arg.to_json_schema_property();
    assert_eq!(name, "format");
    assert_eq!(schema["type"].as_str(), Some("string"));
    let variants = schema["enum"].as_array().expect("enum array");
    assert!(variants.iter().any(|v| v.as_str() == Some("json")));
    assert!(variants.iter().any(|v| v.as_str() == Some("yaml")));
}

#[test]
fn required_cardinality_in_required_array() {
    let spec = CommandSpec {
        args: vec![make_arg(
            "target",
            ArgKind::Option,
            ArgValueType::String,
            Cardinality::Required,
        )],
        ..Default::default()
    };
    let schema = build_input_schema(Some(&spec));
    let required = schema["required"].as_array().expect("required array");
    assert!(required.iter().any(|v| v.as_str() == Some("target")));
}

#[test]
fn optional_not_in_required_array() {
    let spec = CommandSpec {
        args: vec![make_arg(
            "verbose",
            ArgKind::Flag,
            ArgValueType::Bool,
            Cardinality::Optional,
        )],
        ..Default::default()
    };
    let schema = build_input_schema(Some(&spec));
    if let Some(required) = schema.get("required").and_then(|v| v.as_array()) {
        assert!(!required.iter().any(|v| v.as_str() == Some("verbose")));
    }
}

#[test]
fn repeated_flag_count_schema() {
    let arg = make_arg(
        "verbose",
        ArgKind::Flag,
        ArgValueType::Bool,
        Cardinality::Repeated,
    );
    let (_, schema) = arg.to_json_schema_property();
    assert_eq!(schema["type"].as_str(), Some("integer"));
}

#[test]
fn repeated_option_array_schema() {
    let arg = make_arg(
        "tag",
        ArgKind::Option,
        ArgValueType::String,
        Cardinality::Repeated,
    );
    let (_, schema) = arg.to_json_schema_property();
    assert_eq!(schema["type"].as_str(), Some("array"));
    assert_eq!(schema["items"]["type"].as_str(), Some("string"));
}

// ── Argument descriptions reach the generated schema ──────────────────────────
//
// The MCP surface exists for agents, and `description` is the only place an
// agent can read what an argument means. `ArgSpec.help` already holds that
// text; these tests pin that it survives into every generated property,
// including the `Cardinality::Repeated` shapes.

#[test]
fn repeated_positional_property_carries_exact_help_as_description() {
    let arg = ArgSpec {
        name: "skill-ids",
        kind: ArgKind::Positional,
        value_type: ArgValueType::String,
        cardinality: Cardinality::Repeated,
        help: "Skill IDs to remove",
        ..Default::default()
    };
    let (name, schema) = arg.to_json_schema_property();
    assert_eq!(name, "skill-ids");
    assert_eq!(
        schema["description"].as_str(),
        Some("Skill IDs to remove"),
        "repeated positional property must carry its help verbatim, got: {schema}"
    );
}

#[test]
fn repeated_option_property_carries_description() {
    let arg = make_arg(
        "header",
        ArgKind::Option,
        ArgValueType::String,
        Cardinality::Repeated,
    );
    let (_, schema) = arg.to_json_schema_property();
    assert_eq!(schema["type"].as_str(), Some("array"));
    assert_eq!(
        schema["description"].as_str(),
        Some("test arg"),
        "repeated option property must carry its help, got: {schema}"
    );
}

#[test]
fn repeated_flag_property_carries_description() {
    let arg = make_arg(
        "verbose",
        ArgKind::Flag,
        ArgValueType::Bool,
        Cardinality::Repeated,
    );
    let (_, schema) = arg.to_json_schema_property();
    assert_eq!(schema["type"].as_str(), Some("integer"));
    assert_eq!(
        schema["description"].as_str(),
        Some("test arg"),
        "repeated flag property must carry its help, got: {schema}"
    );
}

#[test]
fn empty_help_emits_no_description_key() {
    let arg = ArgSpec {
        name: "tag",
        kind: ArgKind::Option,
        value_type: ArgValueType::String,
        cardinality: Cardinality::Repeated,
        help: "",
        ..Default::default()
    };
    let (_, schema) = arg.to_json_schema_property();
    assert!(
        schema.get("description").is_none(),
        "an empty help must not produce an empty description, got: {schema}"
    );
}

// ── "One or more" is expressible, and reaches `required` ──────────────────────

#[test]
fn repeated_with_min_occurs_one_is_required() {
    let spec = CommandSpec {
        args: vec![ArgSpec {
            name: "skill-ids",
            kind: ArgKind::Positional,
            value_type: ArgValueType::String,
            cardinality: Cardinality::Repeated,
            min_occurs: Some(1),
            help: "Skill IDs to remove",
            ..Default::default()
        }],
        ..Default::default()
    };
    let schema = build_input_schema(Some(&spec));
    let required = schema["required"].as_array().unwrap_or_else(|| {
        panic!("a mandatory repeated arg needs a required array, got: {schema}")
    });
    assert!(
        required.iter().any(|v| v.as_str() == Some("skill-ids")),
        "skill-ids must be listed as required, got: {schema}"
    );
}

#[test]
fn repeated_with_min_occurs_one_sets_min_items() {
    let arg = ArgSpec {
        name: "skill-ids",
        kind: ArgKind::Positional,
        value_type: ArgValueType::String,
        cardinality: Cardinality::Repeated,
        min_occurs: Some(1),
        help: "Skill IDs to remove",
        ..Default::default()
    };
    let (_, schema) = arg.to_json_schema_property();
    assert_eq!(
        schema["minItems"].as_u64(),
        Some(1),
        "a one-or-more array must say so, got: {schema}"
    );
}

#[test]
fn repeated_flag_with_min_occurs_one_sets_minimum() {
    let arg = ArgSpec {
        name: "verbose",
        kind: ArgKind::Flag,
        value_type: ArgValueType::Bool,
        cardinality: Cardinality::Repeated,
        min_occurs: Some(1),
        help: "Verbosity",
        ..Default::default()
    };
    let (_, schema) = arg.to_json_schema_property();
    assert_eq!(schema["type"].as_str(), Some("integer"));
    assert_eq!(
        schema["minimum"].as_u64(),
        Some(1),
        "a mandatory count flag must carry its minimum, got: {schema}"
    );
}

#[test]
fn repeated_without_min_occurs_stays_optional() {
    let spec = CommandSpec {
        args: vec![make_arg(
            "header",
            ArgKind::Option,
            ArgValueType::String,
            Cardinality::Repeated,
        )],
        ..Default::default()
    };
    let schema = build_input_schema(Some(&spec));
    if let Some(required) = schema.get("required").and_then(|v| v.as_array()) {
        assert!(
            !required.iter().any(|v| v.as_str() == Some("header")),
            "zero-or-more repeated args must not become required, got: {schema}"
        );
    }
    assert!(
        schema["properties"]["header"].get("minItems").is_none(),
        "zero-or-more repeated args must not carry minItems, got: {schema}"
    );
}

#[test]
fn repeated_with_min_occurs_zero_stays_optional() {
    let spec = CommandSpec {
        args: vec![ArgSpec {
            name: "header",
            kind: ArgKind::Option,
            value_type: ArgValueType::String,
            cardinality: Cardinality::Repeated,
            min_occurs: Some(0),
            help: "HTTP header",
            ..Default::default()
        }],
        ..Default::default()
    };
    let schema = build_input_schema(Some(&spec));
    if let Some(required) = schema.get("required").and_then(|v| v.as_array()) {
        assert!(
            !required.iter().any(|v| v.as_str() == Some("header")),
            "min_occurs 0 means zero-or-more, got: {schema}"
        );
    }
}

// ── Sweep: no generated property may be description-less ──────────────────────

/// One arg per shape the property builder can produce — every `ArgKind` x
/// `Cardinality` x `ArgValueType` combination that reaches
/// `to_json_schema_property`, each with distinct non-empty help.
fn every_arg_shape() -> Vec<ArgSpec> {
    let mut args = Vec::new();
    let kinds = [ArgKind::Flag, ArgKind::Option, ArgKind::Positional];
    let cardinalities = [
        Cardinality::Required,
        Cardinality::Optional,
        Cardinality::Repeated,
    ];
    let value_types = [
        ArgValueType::String,
        ArgValueType::Bool,
        ArgValueType::Int,
        ArgValueType::Float,
        ArgValueType::Enum(vec!["json", "yaml"]),
    ];

    // `name` is &'static str, so shapes are keyed by a fixed name table.
    const NAMES: [&str; 45] = [
        "a00", "a01", "a02", "a03", "a04", "a05", "a06", "a07", "a08", "a09", "a10", "a11", "a12",
        "a13", "a14", "a15", "a16", "a17", "a18", "a19", "a20", "a21", "a22", "a23", "a24", "a25",
        "a26", "a27", "a28", "a29", "a30", "a31", "a32", "a33", "a34", "a35", "a36", "a37", "a38",
        "a39", "a40", "a41", "a42", "a43", "a44",
    ];

    let mut i = 0;
    for kind in &kinds {
        for cardinality in &cardinalities {
            for value_type in &value_types {
                args.push(ArgSpec {
                    name: NAMES[i],
                    kind: kind.clone(),
                    value_type: value_type.clone(),
                    cardinality: cardinality.clone(),
                    // Alternate so both the zero-or-more and one-or-more
                    // repeated shapes are swept.
                    min_occurs: if i % 2 == 0 { Some(1) } else { None },
                    help: "what this argument means",
                    ..Default::default()
                });
                i += 1;
            }
        }
    }
    args
}

#[test]
fn every_generated_property_has_a_non_empty_description() {
    let spec = CommandSpec {
        summary: "sweep",
        args: every_arg_shape(),
        ..Default::default()
    };
    let schema = build_input_schema(Some(&spec));
    let properties = schema["properties"].as_object().expect("properties object");
    assert_eq!(
        properties.len(),
        45,
        "sweep must cover every generated property"
    );

    // Non-empty, not merely present: a `description: ""` would satisfy a
    // key-exists assertion while telling an agent nothing.
    let offenders: Vec<String> = properties
        .iter()
        .filter(|(_, prop)| {
            prop.get("description")
                .and_then(|d| d.as_str())
                .is_none_or(|d| d.is_empty())
        })
        .map(|(name, prop)| format!("{name}: {prop}"))
        .collect();

    assert!(
        offenders.is_empty(),
        "{} of {} generated properties have a missing or empty description:\n  {}",
        offenders.len(),
        properties.len(),
        offenders.join("\n  ")
    );
}

#[test]
fn long_name_override_used_as_property_key() {
    let arg = ArgSpec {
        name: "v",
        kind: ArgKind::Flag,
        short: Some('v'),
        long: Some("verbose"),
        value_type: ArgValueType::Bool,
        cardinality: Cardinality::Optional,
        default: None,
        conflicts_with: vec![],
        requires: vec![],
        help: "",
        ..Default::default()
    };
    let (name, _) = arg.to_json_schema_property();
    assert_eq!(name, "verbose");
}
