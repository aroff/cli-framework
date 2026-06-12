//! Phase 0 (MCP-Apps extension) tests: per-command UI metadata (CF-1),
//! resource serving with CSP (CF-2), and app-only visibility (CF-3).

use cli_framework::command::{Command, CommandRegistry, UiCsp, UiToolMeta};
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
        ui: None,
        visibility: None,
        execute: noop_execute(),
    }
}

// ── CF-1 — per-command UI metadata ───────────────────────────────────────────

#[test]
fn cf1_tool_descriptor_carries_meta_ui_resource_uri() {
    let mut registry = CommandRegistry::new();
    registry.register(base_cmd("detail", "Open detail view").with_ui(UiToolMeta {
        resource_uri: "ui://es/person/detail".to_string(),
        csp: None,
        prefer_app: true,
    }));

    let tool_registry = McpToolRegistry::from_command_registry(&registry, "es");
    let tools = tool_registry.list_tools();
    let tool = tools
        .iter()
        .find(|t| t.name == "es_detail")
        .expect("es_detail tool present");

    // Serialize the descriptor and assert the exact JSON shape (acceptance).
    let json = serde_json::to_value(tool).unwrap();
    assert_eq!(
        json["_meta"]["ui"]["resourceUri"], "ui://es/person/detail",
        "expected _meta.ui.resourceUri, got: {json}"
    );
    assert_eq!(json["_meta"]["ui"]["preferApp"], true);
}

#[test]
fn cf1_descriptor_without_ui_omits_meta() {
    let mut registry = CommandRegistry::new();
    registry.register(base_cmd("plain", "No UI"));

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
fn cf1_csp_directives_serialize_kebab_case() {
    let mut registry = CommandRegistry::new();
    registry.register(base_cmd("detail", "Open detail view").with_ui(UiToolMeta {
        resource_uri: "ui://es/invoice/detail".to_string(),
        csp: Some(UiCsp {
            default_src: Some("'none'".to_string()),
            style_src: Some("'unsafe-inline'".to_string()),
            img_src: Some("data:".to_string()),
            ..Default::default()
        }),
        prefer_app: false,
    }));

    let tool_registry = McpToolRegistry::from_command_registry(&registry, "es");
    let tools = tool_registry.list_tools();
    let tool = tools.iter().find(|t| t.name == "es_detail").unwrap();
    let json = serde_json::to_value(tool).unwrap();
    let csp = &json["_meta"]["ui"]["csp"];
    assert_eq!(csp["default-src"], "'none'");
    assert_eq!(csp["style-src"], "'unsafe-inline'");
    assert_eq!(csp["img-src"], "data:");
    // Unset directives are omitted.
    assert!(csp.get("script-src").is_none());
}

// ── CF-3 — app-only export policy ────────────────────────────────────────────

#[test]
fn cf3_app_only_tool_is_listed_with_visibility_and_dispatchable() {
    let mut registry = CommandRegistry::new();
    registry.register(
        base_cmd("save", "Save record")
            .with_visibility(vec!["app".to_string()])
            .with_ui(UiToolMeta {
                resource_uri: "ui://es/person/form".to_string(),
                csp: None,
                prefer_app: false,
            }),
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

// ── CF-2 — MCP resource serving (in-process handler) ─────────────────────────

fn handler_with_resources(reg: ResourceRegistry) -> CliFrameworkHandler {
    let cmd_registry = CommandRegistry::new();
    let tool_registry = Arc::new(McpToolRegistry::from_command_registry(&cmd_registry, "es"));
    CliFrameworkHandler::new(tool_registry, McpTransportKind::Http)
        .with_resource_registry(Arc::new(reg))
}

#[test]
fn cf2_read_resource_returns_html_with_csp_meta() {
    let mut reg = ResourceRegistry::new();
    reg.register_static(
        "ui://es/invoice/detail",
        "Invoice detail",
        UiResource::html("<!doctype html><article>invoice</article>").with_csp(UiCsp {
            default_src: Some("'none'".to_string()),
            style_src: Some("'unsafe-inline'".to_string()),
            img_src: Some("data:".to_string()),
            ..Default::default()
        }),
    );
    let handler = handler_with_resources(reg);

    let result = handler
        .read_resource_uri("ui://es/invoice/detail")
        .expect("resource read ok");
    let json = serde_json::to_value(&result).unwrap();
    let content = &json["contents"][0];

    assert_eq!(content["uri"], "ui://es/invoice/detail");
    assert_eq!(content["mimeType"], "text/html");
    assert_eq!(
        content["text"], "<!doctype html><article>invoice</article>",
        "expected HTML body, got: {json}"
    );
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
    reg.register_static(
        "ui://es/invoice/detail",
        "Invoice detail",
        UiResource::html("<x/>"),
    );
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
        uris.contains(&"ui://es/invoice/detail".to_string()),
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
