use crate::spec::arg_spec::{ArgKind, ArgValueType, Cardinality};
use crate::spec::command_tree::CommandSpec;
use serde_json::{json, Value};
use std::collections::BTreeMap;

/// Builds a JSON Schema object for the given spec.
/// Returns `{ "type": "object", "additionalProperties": true }` for spec-less commands.
pub fn build_input_schema(spec: Option<&CommandSpec>) -> Value {
    let Some(spec) = spec else {
        return json!({ "type": "object", "additionalProperties": true });
    };

    let mut properties: BTreeMap<String, Value> = BTreeMap::new();
    let mut required: Vec<String> = Vec::new();

    for arg in &spec.args {
        let prop_name = arg.long.unwrap_or(arg.name).to_string();
        let (_, schema_value) = arg_spec_to_json_schema_property(arg);
        properties.insert(prop_name.clone(), schema_value);
        if arg.cardinality == Cardinality::Required {
            required.push(prop_name);
        }
    }

    let props_value: Value = serde_json::to_value(&properties).unwrap_or(json!({}));

    let mut schema = json!({
        "type": "object",
        "properties": props_value,
    });

    if !required.is_empty() {
        schema["required"] = Value::Array(required.into_iter().map(Value::String).collect());
    }

    schema
}

/// Maps one `ArgSpec` to a `(property_name, schema_value)` pair.
pub fn arg_spec_to_json_schema_property(arg: &crate::spec::arg_spec::ArgSpec) -> (String, Value) {
    let prop_name = arg.long.unwrap_or(arg.name).to_string();

    if arg.cardinality == Cardinality::Repeated {
        let schema_value = if arg.kind == ArgKind::Flag {
            json!({ "type": "integer" })
        } else {
            json!({ "type": "array", "items": { "type": "string" } })
        };
        return (prop_name, schema_value);
    }

    let schema_value = match &arg.value_type {
        ArgValueType::Bool => json!({ "type": "boolean" }),
        ArgValueType::String => json!({ "type": "string" }),
        ArgValueType::Int => json!({ "type": "integer" }),
        ArgValueType::Float => json!({ "type": "number" }),
        ArgValueType::Enum(variants) => json!({
            "type": "string",
            "enum": variants,
        }),
    };
    (prop_name, schema_value)
}
