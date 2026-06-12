//! Basic CLI Example
//!
//! Demonstrates how to create a simple CLI application using cli-framework
//! with basic commands and no AI features.

use cli_framework::prelude::*;
use std::collections::HashMap;
use std::io::{self, Write};
use std::sync::Arc;

// Custom application context
struct MyApp;

impl AppContext for MyApp {}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Create a simple "hello" command
    let hello_command = Command {
        id: Arc::from("hello"),
        spec: Arc::new(CommandSpec {
            summary: "Print a greeting message",
            syntax: Some("hello [name]"),
            category: Some("utilities"),
            ..Default::default()
        }),
        validator: None,
        expose_mcp: false,
        expose_chat: true,
        meta: None,
        visibility: None,
        execute: Arc::new(|_ctx, args| {
            Box::pin(async move {
                let name = args
                    .get("name")
                    .and_then(|v| {
                        if let ArgValue::Str(s) = v {
                            Some(s.as_str())
                        } else {
                            None
                        }
                    })
                    .unwrap_or("World");

                println!("Hello, {}!", name);
                Ok(())
            })
        }),
    };

    // Create an "increment" command that uses app context
    let increment_command = Command {
        id: Arc::from("increment"),
        spec: Arc::new(CommandSpec {
            summary: "Increment and display counter",
            syntax: Some("increment"),
            category: Some("utilities"),
            ..Default::default()
        }),
        validator: None,
        expose_mcp: false,
        expose_chat: true,
        meta: None,
        visibility: None,
        execute: Arc::new(|_ctx, _args| {
            Box::pin(async move {
                // This is a simplified example - in practice, you'd need proper context access
                println!("Counter incremented!");
                Ok(())
            })
        }),
    };

    // Build the CLI application
    let mut builder = AppBuilder::new();
    builder = builder
        .with_version(env!("CARGO_PKG_NAME"), env!("CARGO_PKG_VERSION"))
        .register_command(hello_command)?
        .register_command(increment_command)?;

    let mut app = builder.build(MyApp)?;

    // Simple CLI loop for demonstration
    println!("CLI Framework - Basic Example");
    println!("Available commands: hello, increment");
    println!("Type 'quit' to exit");
    println!();

    let stdin = io::stdin();
    let mut stdout = io::stdout();

    loop {
        print!("> ");
        stdout.flush()?;

        let mut input = String::new();
        stdin.read_line(&mut input)?;
        let input = input.trim();

        if input == "quit" {
            break;
        }

        if input.is_empty() {
            continue;
        }

        // Parse command (basic parsing for demo)
        let parts: Vec<&str> = input.split_whitespace().collect();
        if let Some(command_id) = parts.first() {
            let mut args: HashMap<String, ArgValue> = HashMap::new();
            if parts.len() > 1 {
                args.insert("name".to_string(), ArgValue::Str(parts[1..].join(" ")));
            }

            match app.execute_command(command_id, args).await {
                Ok(()) => {}
                Err(e) => println!("Error: {}", e),
            }
        }
    }

    Ok(())
}
