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
    };
    let (name, _) = arg.to_json_schema_property();
    assert_eq!(name, "verbose");
}
