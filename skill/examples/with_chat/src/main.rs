//! CLI with Chat Example
//!
//! Demonstrates how to enable the built-in `chat` command (feature `chat`).
//! In this rollout phase, `chat` resolves and runs one command per turn,
//! and executes commands against the real application context.

use cli_framework::prelude::*;
use std::sync::Arc;

#[derive(Default)]
struct MyApp {
    counter: u64,
}

impl AppContext for MyApp {
    fn as_any(&self) -> Option<&dyn std::any::Any> {
        Some(self)
    }

    fn as_any_mut(&mut self) -> Option<&mut dyn std::any::Any> {
        Some(self)
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let inc_command = Command {
        id: "inc",
        summary: "Increment a counter in app context",
        syntax: Some("inc"),
        category: Some("data"),
        spec: Some(Arc::new(CommandSpec {
            summary: "Increment a counter in app context",
            ..Default::default()
        })),
        validator: None,
        expose_mcp: false,
        execute: Arc::new(|ctx, _args| {
            Box::pin(async move {
                // Demonstrate that agent-dispatched commands run with the real AppContext.
                // (In this example, MyApp is the concrete context.)
                let app = ctx
                    .as_any_mut()
                    .and_then(|a| a.downcast_mut::<MyApp>())
                    .ok_or_else(|| anyhow::anyhow!("unexpected context type"))?;
                app.counter += 1;
                println!("counter={}", app.counter);
                Ok(())
            })
        }),
    };

    let status_command = Command {
        id: "status",
        summary: "Show a status line",
        syntax: Some("status"),
        category: Some("monitoring"),
        spec: Some(Arc::new(CommandSpec {
            summary: "Show a status line",
            ..Default::default()
        })),
        validator: None,
        expose_mcp: false,
        execute: Arc::new(|_ctx, _args| {
            Box::pin(async move {
                println!("ok");
                Ok(())
            })
        }),
    };

    let mut builder =
        AppBuilder::new().with_version(env!("CARGO_PKG_NAME"), env!("CARGO_PKG_VERSION"));

    // Configure LLM provider from environment (optional at build-time; required at runtime for chat).
    if std::env::var("OPENAI_API_KEY").is_ok() || std::env::var("ANTHROPIC_API_KEY").is_ok() {
        builder = builder
            .with_llm_from_env()?
            .with_ailoop_channel("with_chat");
    }

    builder = builder
        .register_command(inc_command)?
        .register_command(status_command)?;

    let mut app = builder.build(MyApp::default())?;
    app.run().await?;
    Ok(())
}
