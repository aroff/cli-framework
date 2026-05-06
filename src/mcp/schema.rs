use crate::spec::command_tree::CommandSpec;
use serde_json::Value;

#[derive(Debug, Clone, serde::Serialize)]
pub struct McpToolDescriptor {
    pub name: String,
    pub description: String,
    #[serde(rename = "inputSchema")]
    pub input_schema: Value,
}

pub fn command_to_tool_descriptor(
    tool_name: &str,
    summary: &str,
    spec: Option<&CommandSpec>,
) -> McpToolDescriptor {
    McpToolDescriptor {
        name: tool_name.to_string(),
        description: summary.to_string(),
        input_schema: build_input_schema(spec),
    }
}

pub fn build_input_schema(spec: Option<&CommandSpec>) -> Value {
    crate::command_surface::json_schema::build_input_schema(spec)
}

pub fn arg_spec_to_json_schema_property(arg: &crate::spec::arg_spec::ArgSpec) -> (String, Value) {
    crate::command_surface::json_schema::arg_spec_to_json_schema_property(arg)
}
