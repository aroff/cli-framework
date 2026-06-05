use cli_framework::app::AppContext;
use cli_framework::command::Command;
use cli_framework::prelude::*;
use cli_framework::spec::command_tree::CommandSpec;
use std::sync::Arc;

struct Ctx;
impl AppContext for Ctx {}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let mut builder = AppBuilder::new();
    builder = builder
        .with_version("cfw-mcp-stdio-test-server", "0.1.0")
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
            execute: Arc::new(|_ctx, _args| Box::pin(async move { Ok(()) })),
        })?;

    let mut app = builder.build(Ctx)?;
    app.run().await?;
    Ok(())
}
