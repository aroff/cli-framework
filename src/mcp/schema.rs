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
    spec: Option<&crate::spec::command_tree::CommandSpec>,
) -> McpToolDescriptor {
    McpToolDescriptor {
        name: tool_name.to_string(),
        description: summary.to_string(),
        input_schema: crate::command_surface::json_schema::build_input_schema(spec),
    }
}
