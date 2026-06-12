use crate::command::Command;
use serde_json::Value;

/// MCP `tools/list` entry for a single command.
///
/// Beyond the base MCP tool shape (`name`/`description`/`inputSchema`), this
/// carries the MCP-Apps extensions wired by CF-1/CF-3:
/// - `_meta` (`meta`): per-tool passthrough metadata. When the source command
///   has [`crate::command::UiToolMeta`], it is emitted as `_meta.ui` so hosts
///   can open the associated `ui://…` view resource.
/// - `visibility`: optional tags (e.g. `["app"]`) marking an app-only tool.
#[derive(Debug, Clone, serde::Serialize)]
pub struct McpToolDescriptor {
    pub name: String,
    pub description: String,
    #[serde(rename = "inputSchema")]
    pub input_schema: Value,
    /// Per-tool `_meta` passthrough (MCP-Apps). Omitted when empty.
    #[serde(rename = "_meta", skip_serializing_if = "Option::is_none")]
    pub meta: Option<Value>,
    /// Visibility tags such as `["app"]`. Omitted when absent.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub visibility: Option<Vec<String>>,
}

/// Build the `_meta` object for a command, if it carries UI metadata.
///
/// Shape: `{ "ui": { "resourceUri": "...", "csp": {...}, "preferApp": bool } }`.
fn build_meta(cmd: &Command) -> Option<Value> {
    let ui = cmd.ui.as_ref()?;
    Some(serde_json::json!({ "ui": ui }))
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

/// Build a descriptor from a [`Command`], including its MCP-Apps `_meta.ui`
/// and `visibility` (CF-1/CF-3).
pub fn command_to_tool_descriptor_full(tool_name: &str, cmd: &Command) -> McpToolDescriptor {
    McpToolDescriptor {
        name: tool_name.to_string(),
        description: cmd.summary().to_string(),
        input_schema: crate::command_surface::json_schema::build_input_schema(Some(&cmd.spec)),
        meta: build_meta(cmd),
        visibility: cmd.visibility.clone(),
    }
}
