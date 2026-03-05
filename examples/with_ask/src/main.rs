//! CLI with AI Ask Example
//!
//! Demonstrates how to create a CLI application with AI-powered natural language
//! command resolution using the "ask" command.

use cli_framework::prelude::*;
use std::io::{self, Write};

// Custom application context
struct MyApp;

impl AppContext for MyApp {}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Create some example commands
    let deploy_command = Command {
        id: "deploy",
        summary: "Deploy application to specified environment",
        syntax: Some("deploy --env <environment> --version <version>"),
        category: Some("deployment"),
        execute: |_ctx, args| Box::pin(async move {
            let env = args.named.get("env").unwrap_or(&"dev".to_string());
            let version = args.named.get("version").unwrap_or(&"latest".to_string());

            println!("🚀 Deploying version {} to {} environment...", version, env);
            // Simulate deployment
            tokio::time::sleep(tokio::time::Duration::from_secs(2)).await;
            println!("✅ Deployment completed successfully!");
            Ok(())
        }),
    };

    let status_command = Command {
        id: "status",
        summary: "Show system status and health information",
        syntax: Some("status"),
        category: Some("monitoring"),
        execute: |_ctx, _args| Box::pin(async move {
            println!("📊 System Status:");
            println!("   Services: ✅ All running");
            println!("   Database: ✅ Connected");
            println!("   Network:  ✅ Healthy");
            println!("   Uptime:   99.9%");
            Ok(())
        }),
    };

    let logs_command = Command {
        id: "logs",
        summary: "Show application logs",
        syntax: Some("logs --service <service> --lines <count>"),
        category: Some("monitoring"),
        execute: |_ctx, args| Box::pin(async move {
            let service = args.named.get("service").unwrap_or(&"app".to_string());
            let lines = args.named.get("lines")
                .and_then(|s| s.parse().ok())
                .unwrap_or(10);

            println!("📋 Last {} lines of {} logs:", lines, service);
            for i in 1..=lines {
                println!("   {}  [INFO] Log message {}", chrono::Utc::now().format("%H:%M:%S"), i);
            }
            Ok(())
        }),
    };

    // Build the CLI application with LLM support
    let mut builder = AppBuilder::new();

    // Configure LLM provider from environment (if available)
    if let Ok(_) = std::env::var("OPENAI_API_KEY") {
        builder = builder.with_llm_from_env()?;
        println!("🤖 AI ask command enabled (OpenAI)");
    } else if let Ok(_) = std::env::var("ANTHROPIC_API_KEY") {
        builder = builder.with_llm_from_env()?;
        println!("🤖 AI ask command enabled (Anthropic)");
    } else {
        println!("⚠️  No LLM API key found. Ask command will not be available.");
        println!("   Set OPENAI_API_KEY or ANTHROPIC_API_KEY to enable AI features.");
    }

    builder = builder
        .register_command(deploy_command)
        .register_command(status_command)
        .register_command(logs_command);

    let app = builder.build(MyApp)?;

    // Interactive CLI loop
    println!("CLI Framework - AI Ask Example");
    println!("Available commands: deploy, status, logs, ask");
    println!("Try natural language commands like:");
    println!("  - 'ask deploy the app to production'");
    println!("  - 'ask show me the system status'");
    println!("  - 'ask get the last 20 lines of web server logs'");
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