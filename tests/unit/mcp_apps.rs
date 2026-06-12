//! MCP generic-passthrough tests: opaque per-command `_meta` (CF-1), generic
//! resource serving with opaque `_meta` (CF-2), and app-only visibility (CF-3).
//!
//! cli-framework treats `_meta` as opaque passthrough — the consumer
//! (e.g. `entitystore-ui`) owns its shape. These tests therefore build
//! consumer-shaped `_meta` values and assert they survive verbatim on the wire.

use cli_framework::command::{Command, CommandRegistry};
use cli_framework::mcp::resources::{ResourceRegistry, UiResource};
use cli_framework::mcp::{
    CliFrameworkHandler, McpToolExportPolicy, McpToolRegistry, McpTransportKind,
};
use cli_framework::spec::command_tree::CommandSpec;
use cli_framework::spec::value::ArgValue;
use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

#[allow(clippy::type_complexity)]
fn noop_execute() -> Arc<
    dyn for<'a> Fn(
            &'a mut dyn cli_framework::app::AppContext,
            HashMap<String, ArgValue>,
        ) -> Pin<Box<dyn Future<Output = anyhow::Result<()>> + Send + 'a>>
        + Send
        + Sync,
> {
    Arc::new(|_ctx, _args| Box::pin(async { Ok(()) }))
}

fn base_cmd(id: &'static str, summary: &'static str) -> Command {
    Command {
        id: Arc::from(id),
        spec: Arc::new(CommandSpec {
            summary,
            ..Default::default()
        }),
        validator: None,
        expose_mcp: true,
        expose_chat: true,
        meta: None,
        visibility: None,
        execute: noop_execute(),
    }
}

// ── CF-1 — opaque per-command `_meta` passthrough ────────────────────────────

#[test]
fn cf1_tool_descriptor_passes_through_opaque_meta() {
    let mut registry = CommandRegistry::new();
    // The consumer supplies the ENTIRE `_meta` value; cli-framework does not
    // wrap or interpret it. Here a UI-shaped payload built by the consumer.
    registry.register(
        base_cmd("detail", "Open detail view").with_meta(serde_json::json!({
            "ui": { "resourceUri": "ui://es/x/detail" }
        })),
    );

    let tool_registry = McpToolRegistry::from_command_registry(&registry, "es");
    let tools = tool_registry.list_tools();
    let tool = tools
        .iter()
        .find(|t| t.name == "es_detail")
        .expect("es_detail tool present");

    // Serialize the descriptor and assert the consumer's `_meta` survives verbatim.
    let json = serde_json::to_value(tool).unwrap();
    assert_eq!(
        json["_meta"]["ui"]["resourceUri"], "ui://es/x/detail",
        "expected consumer _meta passed through verbatim, got: {json}"
    );
}

#[test]
fn cf1_descriptor_without_meta_omits_meta() {
    let mut registry = CommandRegistry::new();
    registry.register(base_cmd("plain", "No meta"));

    let tool_registry = McpToolRegistry::from_command_registry(&registry, "es");
    let tools = tool_registry.list_tools();
    let tool = tools.iter().find(|t| t.name == "es_plain").unwrap();
    let json = serde_json::to_value(tool).unwrap();
    assert!(
        json.get("_meta").is_none(),
        "expected no _meta key, got: {json}"
    );
    assert!(json.get("visibility").is_none());
}

#[test]
fn cf1_arbitrary_opaque_meta_is_preserved() {
    let mut registry = CommandRegistry::new();
    // A non-UI, arbitrary consumer payload — proves cli-framework is concept-free.
    registry.register(
        base_cmd("detail", "Open detail view").with_meta(serde_json::json!({
            "x_consumer": { "nested": [1, 2, 3], "flag": true }
        })),
    );

    let tool_registry = McpToolRegistry::from_command_registry(&registry, "es");
    let tools = tool_registry.list_tools();
    let tool = tools.iter().find(|t| t.name == "es_detail").unwrap();
    let json = serde_json::to_value(tool).unwrap();
    assert_eq!(
        json["_meta"]["x_consumer"]["nested"],
        serde_json::json!([1, 2, 3])
    );
    assert_eq!(json["_meta"]["x_consumer"]["flag"], true);
}

// ── CF-3 — app-only export policy ────────────────────────────────────────────

#[test]
fn cf3_app_only_tool_is_listed_with_visibility_and_dispatchable() {
    let mut registry = CommandRegistry::new();
    registry.register(
        base_cmd("save", "Save record")
            .with_visibility(vec!["app".to_string()])
            .with_meta(serde_json::json!({
                "ui": { "resourceUri": "ui://es/x/form" }
            })),
    );

    let tool_registry = McpToolRegistry::from_command_registry_with_policy(
        &registry,
        "es",
        McpToolExportPolicy::ExposeMcpOnly,
    );
    let tools = tool_registry.list_tools();
    let tool = tools
        .iter()
        .find(|t| t.name == "es_save")
        .expect("app-only tool still listed");

    let json = serde_json::to_value(tool).unwrap();
    assert_eq!(
        json["visibility"],
        serde_json::json!(["app"]),
        "expected visibility:[\"app\"], got: {json}"
    );

    // Still dispatchable: resolvable in the registry (the tools/call path).
    assert!(
        tool_registry.resolve_tool("es_save").is_some(),
        "app-only tool must remain dispatchable via tools/call"
    );
}

// ── CF-2 — generic MCP resource serving (in-process handler) ─────────────────

fn handler_with_resources(reg: ResourceRegistry) -> CliFrameworkHandler {
    let cmd_registry = CommandRegistry::new();
    let tool_registry = Arc::new(McpToolRegistry::from_command_registry(&cmd_registry, "es"));
    CliFrameworkHandler::new(tool_registry, McpTransportKind::Http)
        .with_resource_registry(Arc::new(reg))
}

#[test]
fn cf2_read_resource_returns_html_with_opaque_meta() {
    let mut reg = ResourceRegistry::new();
    // The consumer attaches an opaque `_meta` carrying a UI-shaped object; the
    // framework emits it verbatim at `contents[]._meta`.
    reg.register_static(
        "ui://es/x/detail",
        "X detail",
        UiResource::html("<!doctype html><article>x</article>").with_meta(serde_json::json!({
            "ui": {
                "csp": {
                    "default-src": "'none'",
                    "style-src": "'unsafe-inline'",
                    "img-src": "data:"
                }
            }
        })),
    );
    let handler = handler_with_resources(reg);

    let result = handler
        .read_resource_uri("ui://es/x/detail")
        .expect("resource read ok");
    let json = serde_json::to_value(&result).unwrap();
    let content = &json["contents"][0];

    assert_eq!(content["uri"], "ui://es/x/detail");
    assert_eq!(content["mimeType"], "text/html");
    assert_eq!(
        content["text"], "<!doctype html><article>x</article>",
        "expected HTML body, got: {json}"
    );
    // The opaque `_meta` lands verbatim at contents[]._meta.
    assert_eq!(content["_meta"]["ui"]["csp"]["default-src"], "'none'");
    assert_eq!(
        content["_meta"]["ui"]["csp"]["style-src"],
        "'unsafe-inline'"
    );
    assert_eq!(content["_meta"]["ui"]["csp"]["img-src"], "data:");
}

#[test]
fn cf2_read_unregistered_resource_is_not_found() {
    let handler = handler_with_resources(ResourceRegistry::new());
    let err = handler
        .read_resource_uri("ui://es/missing")
        .expect_err("missing resource must error");
    assert!(
        err.message.contains("MCP_RESOURCE_NOT_FOUND"),
        "expected MCP_RESOURCE_NOT_FOUND, got: {}",
        err.message
    );
}

#[test]
fn cf2_list_resources_includes_registered_uri() {
    let mut reg = ResourceRegistry::new();
    reg.register_static("ui://es/x/detail", "X detail", UiResource::html("<x/>"));
    let handler = handler_with_resources(reg);

    let result = handler.list_resources_result();
    let json = serde_json::to_value(&result).unwrap();
    let uris: Vec<String> = json["resources"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["uri"].as_str().unwrap().to_string())
        .collect();
    assert!(
        uris.contains(&"ui://es/x/detail".to_string()),
        "expected listed resource, got: {uris:?}"
    );
}

#[test]
fn cf2_provider_closure_is_invoked_dynamically() {
    let mut reg = ResourceRegistry::new();
    reg.register(
        "ui://es/dynamic",
        "Dynamic",
        None,
        Some("text/html".to_string()),
        |uri| Some(UiResource::html(format!("<p>{uri}</p>"))),
    );
    let handler = handler_with_resources(reg);

    let result = handler.read_resource_uri("ui://es/dynamic").unwrap();
    let json = serde_json::to_value(&result).unwrap();
    assert_eq!(json["contents"][0]["text"], "<p>ui://es/dynamic</p>");
}
