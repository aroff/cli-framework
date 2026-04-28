//! Basic CLI Example
//!
//! Demonstrates how to create a simple CLI application using cli-framework
//! with basic commands and no AI features.

use cli_framework::prelude::*;
use std::io::{self, Write};
use std::sync::Arc;

// Custom application context
struct MyApp;

impl AppContext for MyApp {}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Create a simple "hello" command
    let hello_command = Command {
        id: "hello",
        summary: "Print a greeting message",
        syntax: Some("hello [name]"),
        category: Some("utilities"),
        spec: None,
        validator: None,
        execute: Arc::new(|_ctx, args| {
            Box::pin(async move {
                let name = args
                    .positional
                    .get(0)
                    .map(String::as_str)
                    .unwrap_or("World");

                println!("Hello, {}!", name);
                Ok(())
            })
        }),
    };

    // Create an "increment" command that uses app context
    let increment_command = Command {
        id: "increment",
        summary: "Increment and display counter",
        syntax: Some("increment"),
        category: Some("utilities"),
        spec: None,
        validator: None,
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
        if let Some(command_id) = parts.get(0) {
            let args = if parts.len() > 1 {
                CommandArgs {
                    positional: parts[1..].iter().map(|s| s.to_string()).collect(),
                    named: std::collections::HashMap::new(),
                }
            } else {
                CommandArgs::default()
            };

            match app.execute_command(command_id, args).await {
                Ok(()) => {}
                Err(e) => println!("Error: {}", e),
            }
        }
    }

    Ok(())
}
