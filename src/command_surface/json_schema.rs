use crate::spec::command_tree::CommandSpec;
use serde_json::{json, Value};

/// Builds a JSON Schema object for the given spec.
/// Returns `{ "type": "object", "additionalProperties": true }` for spec-less commands.
pub fn build_input_schema(spec: Option<&CommandSpec>) -> Value {
    let Some(spec) = spec else {
        return json!({ "type": "object", "additionalProperties": true });
    };

    spec.to_json_schema()
}
