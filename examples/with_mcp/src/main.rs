//! MCP Server Example
//!
//! Demonstrates how to start the CLI framework in MCP server mode.
//! Run with: cargo run --example with_mcp --features "mcp-server clap-dispatch" -- --mcp-serve --mcp-port 8080

use cli_framework::prelude::*;
use std::sync::Arc;

struct MyApp;
impl AppContext for MyApp {}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    env_logger::init();

    let mut builder = AppBuilder::new();
    builder = builder
        .with_version("my-mcp-app", "0.1.0")
        .register_command(Command {
            id: "hello",
            summary: "Say hello to the world",
            syntax: Some("hello"),
            category: Some("greetings"),
            spec: Some(Arc::new(CommandSpec {
                summary: "Say hello to the world",
                args: vec![ArgSpec {
                    name: "name",
                    kind: cli_framework::spec::arg_spec::ArgKind::Option,
                    short: None,
                    long: None,
                    value_type: cli_framework::spec::arg_spec::ArgValueType::String,
                    cardinality: cli_framework::spec::arg_spec::Cardinality::Optional,
                    default: None,
                    conflicts_with: vec![],
                    requires: vec![],
                    help: "Name to greet",
                }],
                ..Default::default()
            })),
            validator: None,
            execute: Arc::new(|_ctx, args| {
                Box::pin(async move {
                    let name = args
                        .named
                        .get("name")
                        .map(String::as_str)
                        .unwrap_or("World");
                    println!("Hello, {}!", name);
                    Ok(())
                })
            }),
        })?
        .register_command(Command {
            id: "status",
            summary: "Show application status",
            syntax: Some("status"),
            category: Some("info"),
            spec: None,
            validator: None,
            execute: Arc::new(|_ctx, _args| {
                Box::pin(async move {
                    println!("Status: OK");
                    Ok(())
                })
            }),
        })?;

    let mut app = builder.build(MyApp)?;
    app.run().await?;
    Ok(())
}
