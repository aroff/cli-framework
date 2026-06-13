use cli_framework::app::AppContext;
use cli_framework::command::Command;
use cli_framework::mcp::resources::{ResourceRegistry, UiResource};
use cli_framework::prelude::*;
use cli_framework::spec::command_tree::CommandSpec;
use std::sync::Arc;

struct Ctx;
impl AppContext for Ctx {}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Populate a resource registry and hand it to the serve path via the
    // public `with_mcp_resource_registry` slot (CF-6). This mirrors how an
    // MCP-Apps binding registers `ui://…` providers and serves them.
    let mut resources = ResourceRegistry::new();
    resources.register_static(
        "ui://cfw-test/index.html",
        "Test UI shell",
        UiResource::html("<!doctype html><title>cfw-test</title><main>hi</main>"),
    );

    let mut builder = AppBuilder::new();
    builder = builder
        .with_version("cfw-mcp-stdio-test-server", "0.1.0")
        .with_mcp_resource_registry(Arc::new(resources))
        .register_command(Command {
            id: Arc::from("ping"),
            spec: Arc::new(CommandSpec {
                summary: "Ping (no stdout)",
                syntax: Some("ping"),
                category: Some("test"),
                args: vec![],
                ..Default::default()
            }),
            validator: None,
            expose_mcp: true,
            expose_chat: true,
            meta: None,
            visibility: None,
            execute: Arc::new(|_ctx, _args| Box::pin(async move { Ok(()) })),
        })?;

    let mut app = builder.build(Ctx)?;
    app.run().await?;
    Ok(())
}
