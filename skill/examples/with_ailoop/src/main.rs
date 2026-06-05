//! CLI with ailoop Integration Example
//!
//! Demonstrates how to create a CLI application with ailoop-core integration
//! for human-in-the-loop confirmations and interactions.

use cli_framework::prelude::*;
use std::collections::HashMap;
use std::io::{self, Write};
use std::sync::Arc;

// Custom application context with ailoop support
struct MyApp;

impl AppContext for MyApp {}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Create a command that requires confirmation
    let deploy_command = Command {
        id: Arc::from("deploy"),
        spec: Arc::new(CommandSpec {
            summary: "Deploy application (requires confirmation)",
            syntax: Some("deploy --env <environment>"),
            category: Some("deployment"),
            ..Default::default()
        }),
        validator: None,
        expose_mcp: false,
        expose_chat: true,
        execute: Arc::new(|_ctx, args| {
            Box::pin(async move {
                let env = args
                    .get("env")
                    .and_then(|v| {
                        if let ArgValue::Str(s) = v {
                            Some(s.as_str())
                        } else {
                            None
                        }
                    })
                    .unwrap_or("staging");

                println!("🚀 Preparing to deploy to {} environment...", env);

                let ailoop = cli_framework::ailoop::AiloopClient::new()?;
                let confirmed = ailoop
                    .request_confirmation(
                        &format!("Deploy application to {} environment", env),
                        Some("This will update live systems and may cause downtime"),
                    )
                    .await?;

                if !confirmed {
                    println!("❌ Deployment cancelled by user");
                    return Ok(());
                }

                println!("✅ Deployment confirmed and started!");
                // Simulate deployment
                tokio::time::sleep(tokio::time::Duration::from_secs(3)).await;
                println!("🎉 Deployment completed successfully!");
                Ok(())
            })
        }),
    };

    // Create a command that asks questions
    let configure_command = Command {
        id: Arc::from("configure"),
        spec: Arc::new(CommandSpec {
            summary: "Configure application settings",
            syntax: Some("configure"),
            category: Some("setup"),
            ..Default::default()
        }),
        validator: None,
        expose_mcp: false,
        expose_chat: true,
        execute: Arc::new(|_ctx, _args| {
            Box::pin(async move {
                println!("⚙️  Configuring application...");

                let ailoop = cli_framework::ailoop::AiloopClient::new()?;
                let db_type = ailoop
                    .ask_question(
                        "Which database type would you like to use?",
                        Some(vec![
                            "PostgreSQL".to_string(),
                            "MySQL".to_string(),
                            "SQLite".to_string(),
                        ]),
                    )
                    .await?;

                let cache_enabled = ailoop
                    .request_confirmation(
                        "Enable Redis caching?",
                        Some("Caching improves performance but requires Redis server"),
                    )
                    .await?;

                println!("📝 Configuration:");
                println!("   Database: {}", db_type);
                println!(
                    "   Caching: {}",
                    if cache_enabled { "Enabled" } else { "Disabled" }
                );

                ailoop
                    .send_notification("Application configuration completed", Some("normal"))
                    .await?;

                println!("✅ Configuration completed!");
                Ok(())
            })
        }),
    };

    // Build the CLI application with ailoop integration
    let mut builder = AppBuilder::new();

    // Configure ailoop (this would connect to ailoop server in production)
    builder = builder.with_ailoop_channel("cli-framework-demo");

    builder = builder
        .with_version(env!("CARGO_PKG_NAME"), env!("CARGO_PKG_VERSION"))
        .register_command(deploy_command)?
        .register_command(configure_command)?;

    let mut app = builder.build(MyApp)?;

    // Interactive CLI loop
    println!("CLI Framework - ailoop Integration Example");
    println!("Available commands: deploy, configure");
    println!();
    println!("This example demonstrates ailoop integration for:");
    println!("  - Human confirmation for critical operations");
    println!("  - Interactive configuration with questions");
    println!("  - Status notifications");
    println!();
    println!("Note: This is a demo - ailoop confirmations are simulated.");
    println!("In production, this would connect to a real ailoop server.");
    println!();
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
            // Parse --key value pairs from remaining parts
            let rest = &parts[1..];
            let mut i = 0;
            while i < rest.len() {
                if let Some(key) = rest[i].strip_prefix("--") {
                    if i + 1 < rest.len() {
                        args.insert(key.to_string(), ArgValue::Str(rest[i + 1].to_string()));
                        i += 2;
                    } else {
                        args.insert(key.to_string(), ArgValue::Bool(true));
                        i += 1;
                    }
                } else {
                    i += 1;
                }
            }

            match app.execute_command(command_id, args).await {
                Ok(()) => {}
                Err(e) => println!("Error: {}", e),
            }
        }
    }

    Ok(())
}
