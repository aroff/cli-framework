//! CLI with Plugins Example
//!
//! Demonstrates how to create a CLI application that loads third-party commands
//! from plugin manifests and registry files.

use cli_framework::plugin::manifest::{CommandExecution, PluginCommand};
use cli_framework::prelude::*;
use std::io::{self, Write};

// Custom application context
struct MyApp;

impl AppContext for MyApp {}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Create a basic command
    let builtin_command = Command {
        id: "builtin",
        summary: "Execute a built-in command",
        syntax: Some("builtin <message>"),
        category: Some("builtins"),
        execute: |_ctx, args| {
            Box::pin(async move {
                let message = args
                    .positional
                    .get(0)
                    .map(String::as_str)
                    .unwrap_or("Hello from built-in command!");
                println!("🔧 Built-in: {}", message);
                Ok(())
            })
        },
    };

    // Build the CLI application with plugin support
    let mut builder = AppBuilder::new();

    // Configure plugin registry path (create a sample registry)
    let registry_path = std::path::PathBuf::from("plugin-registry.toml");
    create_sample_registry(&registry_path).await?;
    builder = builder.with_plugin_registry_path(registry_path);

    builder = builder
        .with_version(env!("CARGO_PKG_NAME"), env!("CARGO_PKG_VERSION"))
        .register_command(builtin_command);

    let mut app = builder.build(MyApp)?;

    // Interactive CLI loop
    println!("CLI Framework - Plugins Example");
    println!("Available commands: builtin (and any loaded plugins)");
    println!();
    println!("This example demonstrates plugin loading. In a real application,");
    println!("plugins would be installed to ~/.config/cli-framework/registry.toml");
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

/// Create a sample plugin registry for demonstration
async fn create_sample_registry(path: &std::path::Path) -> anyhow::Result<()> {
    use cli_framework::plugin::PluginRegistryConfig;

    // Create a sample plugin manifest
    let manifest = cli_framework::plugin::PluginManifest {
        name: "sample-plugin".to_string(),
        version: "1.0.0".to_string(),
        description: Some("Sample plugin for demonstration".to_string()),
        author: Some("CLI Framework".to_string()),
        commands: vec![PluginCommand {
            id: "sample-hello".to_string(),
            name: "Sample Hello".to_string(),
            description: "Print a hello message from plugin".to_string(),
            syntax: Some("sample-hello".to_string()),
            category: Some("samples".to_string()),
            execution: CommandExecution::Subprocess {
                command: "echo".to_string(),
                args: vec!["Hello from plugin!".to_string()],
                cwd: None,
            },
        }],
    };

    // Save the manifest
    let manifest_path = path.with_file_name("sample-plugin.json");
    manifest.save_to_file(&manifest_path).await?;

    // Create registry config
    let mut config = PluginRegistryConfig::default();
    config.add_plugin(
        "sample".to_string(),
        cli_framework::plugin::PluginEntry {
            name: "Sample Plugin".to_string(),
            version: "1.0.0".to_string(),
            manifest_path: manifest_path.to_string_lossy().to_string(),
            enabled: true,
            priority: 0,
        },
    );

    // Save registry
    config.save_to_file(path).await?;

    println!("📦 Created sample plugin registry at: {}", path.display());
    println!("   Plugin: sample-hello");

    Ok(())
}
