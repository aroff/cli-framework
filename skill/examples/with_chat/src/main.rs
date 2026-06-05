//! CLI with Chat Example
//!
//! Demonstrates how to enable the built-in `chat` command (feature `chat`).
//! In this rollout phase, `chat` runs an embedded agent and exposes only
//! the app's registered commands as tools, executed against the real AppContext.

use cli_framework::prelude::*;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

struct MyApp;

impl AppContext for MyApp {}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let counter = Arc::new(AtomicU64::new(0));
    let counter_for_inc = Arc::clone(&counter);

    let inc_command = Command {
        id: Arc::from("inc"),
        spec: Arc::new(CommandSpec {
            summary: "Increment a counter in app context",
            ..Default::default()
        }),
        validator: None,
        expose_mcp: false,
        expose_chat: true,
        execute: Arc::new(move |_ctx, _args| {
            let counter = Arc::clone(&counter_for_inc);
            Box::pin(async move {
                let val = counter.fetch_add(1, Ordering::SeqCst) + 1;
                println!("counter={}", val);
                Ok(())
            })
        }),
    };

    let status_command = Command {
        id: Arc::from("status"),
        spec: Arc::new(CommandSpec {
            summary: "Show a status line",
            ..Default::default()
        }),
        validator: None,
        expose_mcp: false,
        expose_chat: true,
        execute: Arc::new(|_ctx, _args| {
            Box::pin(async move {
                println!("ok");
                Ok(())
            })
        }),
    };

    let mut builder =
        AppBuilder::new().with_version(env!("CARGO_PKG_NAME"), env!("CARGO_PKG_VERSION"));

    builder = builder
        .register_command(inc_command)?
        .register_command(status_command)?;

    let mut app = builder.build(MyApp)?;
    app.run().await?;
    let _ = counter;
    Ok(())
}
