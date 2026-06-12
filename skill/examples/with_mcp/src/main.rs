//! MCP Server Example
//!
//! Demonstrates two MCP server patterns:
//!
//! 1. **`mcp serve` subcommand** (recommended) — first-class named command with `--help` support:
//!    ```bash
//!    cargo run --example with_mcp --features "mcp-server" -- mcp serve
//!    cargo run --example with_mcp --features "mcp-server" -- mcp serve --host 0.0.0.0 --port 9090 --path /mcp
//!    cargo run --example with_mcp --features "mcp-server" -- mcp --help
//!    cargo run --example with_mcp --features "mcp-server" -- mcp serve --help
//!    ```
//!
//! 2. **Embedded-mount mode** — MCP is nested into an existing Axum router on the same port:
//!    ```bash
//!    cargo run --example with_mcp --features "mcp-server" -- --embedded-mcp
//!    ```

use axum::routing::get;
use cli_framework::command::{Command, CommandRegistry};
use cli_framework::mcp::{build_mcp_axum_router, McpToolExportPolicy};
use cli_framework::prelude::*;
use cli_framework::security::CommandRiskPolicy;
use cli_framework::spec::arg_spec::{ArgKind, ArgSpec, ArgValueType, Cardinality};
use cli_framework::spec::value::ArgValue;
use std::collections::HashMap;
use std::sync::Arc;

struct MyApp;
impl AppContext for MyApp {}

fn make_hello_command() -> Command {
    Command {
        id: Arc::from("hello"),
        spec: Arc::new(CommandSpec {
            summary: "Say hello to the world",
            syntax: Some("hello"),
            category: Some("greetings"),
            args: vec![ArgSpec {
                name: "name",
                kind: ArgKind::Option,
                long: Some("name"),
                value_type: ArgValueType::String,
                cardinality: Cardinality::Optional,
                help: "Name to greet",
                ..Default::default()
            }],
            ..Default::default()
        }),
        validator: None,
        expose_mcp: true,
        expose_chat: true,
        meta: None,
        visibility: None,
        execute: Arc::new(|_ctx, args: HashMap<String, ArgValue>| {
            Box::pin(async move {
                let name = match args.get("name") {
                    Some(ArgValue::Str(s)) => s.as_str(),
                    _ => "World",
                };
                println!("Hello, {}!", name);
                Ok(())
            })
        }),
    }
}

fn make_status_command() -> Command {
    Command {
        id: Arc::from("status"),
        spec: Arc::new(CommandSpec {
            summary: "Show application status",
            syntax: Some("status"),
            category: Some("info"),
            ..Default::default()
        }),
        validator: None,
        expose_mcp: false,
        expose_chat: true,
        meta: None,
        visibility: None,
        execute: Arc::new(|_ctx, _args| {
            Box::pin(async move {
                println!("Status: OK");
                Ok(())
            })
        }),
    }
}

fn build_registry() -> anyhow::Result<CommandRegistry> {
    let mut registry = CommandRegistry::new();
    registry.register(make_hello_command());
    registry.register(make_status_command());
    Ok(registry)
}

/// Embedded-mount mode: MCP routes are nested into an existing Axum router.
///
/// The host application owns the `TcpListener`, TLS termination, global middleware,
/// and graceful shutdown. `build_mcp_axum_router` returns a plain `axum::Router`
/// that can be composed with any other routes via `.merge()` or `.nest()`.
///
/// The path prefix (`"/mcp"`) is forwarded verbatim; the caller is responsible
/// for preventing prefix collisions with other routes.
async fn run_embedded_mcp() -> anyhow::Result<()> {
    let registry = build_registry()?;

    // Build the MCP router fragment — no port is bound here.
    // Use ExposeMcpOnly to expose only commands flagged with expose_mcp: true.
    let mcp_router = build_mcp_axum_router(
        &registry,
        "my-mcp-app",
        "/mcp",
        CommandRiskPolicy::default(),
        McpToolExportPolicy::AllCommands,
    );

    // Compose with host-application routes. The caller owns the listener.
    let app = axum::Router::new()
        .route("/health", get(|| async { "OK" }))
        .merge(mcp_router);

    let listener = tokio::net::TcpListener::bind("127.0.0.1:8081").await?;
    println!("Embedded MCP listening on http://127.0.0.1:8081/mcp");
    println!("Health check on http://127.0.0.1:8081/health");
    axum::serve(listener, app).await?;
    Ok(())
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    env_logger::init();

    let args: Vec<String> = std::env::args().collect();
    if args.iter().any(|a| a == "--embedded-mcp") {
        // Embedded-mount pattern: share one listener between MCP and other routes.
        return run_embedded_mcp().await;
    }

    // Standalone mode: use `mcp serve` subcommand to start MCP server.
    let builder = AppBuilder::new()
        .with_version("my-mcp-app", "0.1.0")
        .register_command(make_hello_command())?
        .register_command(make_status_command())?;

    let mut app = builder.build(MyApp)?;
    app.run().await?;
    Ok(())
}
