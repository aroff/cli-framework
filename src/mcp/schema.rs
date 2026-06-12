use crate::command::Command;
use serde_json::Value;

/// MCP `tools/list` entry for a single command.
///
/// Beyond the base MCP tool shape (`name`/`description`/`inputSchema`), this
/// carries two generic MCP fields:
/// - `_meta` (`meta`): opaque per-tool passthrough metadata, sourced verbatim
///   from [`crate::command::Command::meta`]. cli-framework does not interpret
///   it; the consumer owns the entire shape.
/// - `visibility`: optional tags (e.g. `["app"]`) marking an app-only tool —
///   the one field cli-framework acts on, for app-only dispatch behavior.
#[derive(Debug, Clone, serde::Serialize)]
pub struct McpToolDescriptor {
    pub name: String,
    pub description: String,
    #[serde(rename = "inputSchema")]
    pub input_schema: Value,
    /// Opaque per-tool `_meta` passthrough. Omitted when absent.
    #[serde(rename = "_meta", skip_serializing_if = "Option::is_none")]
    pub meta: Option<Value>,
    /// Visibility tags such as `["app"]`. Omitted when absent.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub visibility: Option<Vec<String>>,
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
        meta: None,
        visibility: None,
    }
}

/// Build a descriptor from a [`Command`], passing its opaque `_meta` and
/// `visibility` through unchanged.
pub fn command_to_tool_descriptor_full(tool_name: &str, cmd: &Command) -> McpToolDescriptor {
    McpToolDescriptor {
        name: tool_name.to_string(),
        description: cmd.summary().to_string(),
        input_schema: crate::command_surface::json_schema::build_input_schema(Some(&cmd.spec)),
        meta: cmd.meta.clone(),
        visibility: cmd.visibility.clone(),
    }
}
