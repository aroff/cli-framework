# CLI Framework

A pure CLI framework with AI-powered command resolution and plugin system for Rust. Build powerful CLI applications with natural language command processing, third-party plugin support, and human-in-the-loop interactions.

## Features

- **🤖 AI Ask Command**: Natural language command resolution using OpenAI/Anthropic LLMs
- **🔌 Plugin System**: Registry-based third-party command loading from manifest files
- **👥 Human-in-the-Loop**: ailoop-core integration for confirmations and interactive prompts
- **🔄 Command Registry**: Centralized command management with metadata collection
- **📊 Rich CLI Output**: Tables, JSON, progress indicators, and formatted messages
- **⚡ Async-First**: Built on Tokio for high-performance async operations
- **🛠️ Extensible**: Easy to add new LLM providers, plugins, and integrations

## Quick Start

Add to your `Cargo.toml`:

```toml
[dependencies]
cli-framework = { path = "../cli-framework" }  # or from crates.io when published
anyhow = "1.0"
tokio = { version = "1", features = ["full"] }
```

Basic CLI application:

```rust
use cli_framework::prelude::*;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Create a command
    let hello_command = Command {
        id: "hello",
        summary: "Print a greeting",
        syntax: Some("hello [name]"),
        category: Some("utilities"),
        execute: |ctx, args| Box::pin(async move {
            let name = args.positional.get(0).unwrap_or(&"World".to_string());
            println!("Hello, {}!", name);
            Ok(())
        }),
    };

    // Build and use the app
    let mut builder = AppBuilder::new();
    builder = builder.register_command(hello_command);

    let app = builder.build(MyContext)?;

    // Execute commands
    app.execute_command("hello", CommandArgs {
        positional: vec!["Alice".to_string()],
        named: std::collections::HashMap::new(),
    }).await?;

    Ok(())
}

struct MyContext;
impl AppContext for MyContext {}
```

## AI Ask Command

Enable natural language command resolution:

```rust
use cli_framework::prelude::*;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Set up LLM provider
    std::env::set_var("OPENAI_API_KEY", "your-api-key");

    let mut builder = AppBuilder::new();
    builder = builder.with_llm_from_env()?;  // Auto-detects from env vars

    // Register commands (ask command is added automatically)
    builder = builder.register_command(deploy_command);

    let app = builder.build(MyContext)?;

    // Users can now type natural language:
    // "ask deploy the app to production"
    // "ask show me the system status"

    Ok(())
}
```

## Quick Start

Add to your `Cargo.toml`:

```toml
[dependencies]
tui-framework = { path = "../tui-framework" }  # or from crates.io when published
anyhow = "1.0"
async-trait = "0.1"
tokio = { version = "1", features = ["full"] }
```

Minimal example:

```rust
use async_trait::async_trait;
use tui_framework::prelude::*;
use tui_framework::view::{View, ViewResult, HelpItem, Theme};
use tui_framework::widget::GridView;
use tui_framework::data_source::DataSource;
use crossterm::event::Event;
use ratatui::layout::Rect;
use ratatui::Frame;
use anyhow::Result;

// Your data model
#[derive(Clone, Debug)]
struct Item {
    name: String,
    status: String,
}

// Implement DataSource for your data
struct ItemDataSource {
    items: Vec<Item>,
}

#[async_trait]
impl DataSource for ItemDataSource {
    type Row = Item;

    fn len(&self) -> usize {
        self.items.len()
    }

    fn get(&self, index: usize) -> Option<&Self::Row> {
        self.items.get(index)
    }

    async fn refresh(&mut self, _ctx: &dyn AppContext) -> Result<()> {
        // Fetch your data here (e.g., from an API)
        Ok(())
    }
}

// Create a view
struct MyView {
    grid: GridView<ItemDataSource>,
}

impl MyView {
    fn new() -> Self {
        let data_source = ItemDataSource {
            items: vec![
                Item { name: "Item 1".to_string(), status: "Active".to_string() },
                Item { name: "Item 2".to_string(), status: "Inactive".to_string() },
            ],
        };
        let theme = Theme::default();
        let grid = GridView::new(data_source, theme)
            .with_formatter(|item: &Item| -> Vec<String> {
                vec![item.name.clone(), item.status.clone()]
            });
        Self { grid }
    }
}

#[async_trait]
impl View for MyView {
    fn id(&self) -> &'static str { "my.view" }
    fn title(&self) -> &'static str { "My View" }

    fn render(&mut self, f: &mut Frame, area: Rect, _ctx: &dyn AppContext) {
        self.grid.render(f, area);
    }

    async fn handle_event(&mut self, _event: &Event, _ctx: &mut dyn AppContext) -> ViewResult {
        ViewResult::Ignored
    }

    fn help_items(&self) -> Vec<HelpItem> {
        vec![
            HelpItem { key: "q".to_string(), description: "Quit".to_string() },
            HelpItem { key: "?".to_string(), description: "Help".to_string() },
        ]
    }
}

// App context (holds your application state)
struct MyContext;
impl AppContext for MyContext {}

// Build and run
#[tokio::main]
async fn main() -> Result<()> {
    let mut builder = AppBuilder::new();
    builder = builder
        .register_view(MyView::new())
        .map_view_slot(ViewSlot::Slot1, "my.view");

    let ctx = MyContext;
    let mut app = builder.build(ctx)?;
    app.run().await?;

    Ok(())
}
```

## Core Concepts

### Commands

Commands are executable operations in your CLI application. Each command has metadata for AI resolution:

```rust
use cli_framework::prelude::*;

let deploy_command = Command {
    id: "deploy",
    summary: "Deploy application to specified environment",
    syntax: Some("deploy --env <environment> --version <version>"),
    category: Some("deployment"),
    execute: |ctx, args| Box::pin(async move {
        let env = args.named.get("env").unwrap_or(&"dev".to_string());
        println!("🚀 Deploying to {}...", env);
        Ok(())
    }),
};

builder = builder.register_command(deploy_command);
```

### AppContext

`AppContext` holds your application's state and services:

```rust
struct MyAppContext {
    api_client: reqwest::Client,
    config: AppConfig,
    database: DatabaseConnection,
}

impl AppContext for MyAppContext {}
```

### Plugin System

Load third-party commands from manifest files:

```toml
# plugin-registry.toml
[plugins.sample]
name = "Sample Plugin"
version = "1.0.0"
manifest_path = "/path/to/plugin.json"
enabled = true
```

```json
// plugin.json
{
  "name": "sample-plugin",
  "commands": [
    {
      "id": "sample-hello",
      "name": "Sample Hello",
      "description": "Print hello from plugin",
      "syntax": "sample-hello",
      "category": "demo"
    }
  ]
}
```

### ailoop Integration

Add human-in-the-loop confirmations:

```rust
// Implement AiloopContext
impl cli_framework::ailoop::AiloopContext for MyAppContext {
    fn ailoop_client(&self) -> &cli_framework::ailoop::AiloopClient {
        // Return configured ailoop client
    }
}

// Use in commands
let confirmed = ctx.ailoop_client()
    .request_confirmation("Deploy to production?", Some("This affects live users"))
    .await?;
```

## Common Patterns

### AI Command Resolution

Enable natural language command processing:

```rust
// Environment variables
std::env::set_var("OPENAI_API_KEY", "sk-...");
std::env::set_var("LLM_PROVIDER", "openai");
std::env::set_var("LLM_MODEL", "gpt-4");

// Configure in builder
let builder = AppBuilder::new()
    .with_llm_from_env()?  // Automatically detects provider from env
    .register_command(my_command);

// Users can now type:
// "ask deploy to production"
// "ask show system status"
// "ask restart the web server"
```

### Plugin Loading

Load third-party commands at startup:

```rust
let builder = AppBuilder::new()
    .with_plugin_registry_path("~/.config/myapp/plugins.toml".into())
    .register_command(builtin_command);

// Plugins are automatically loaded and their commands registered
```

### Human Confirmations

Use ailoop for critical operations:

```rust
async fn dangerous_command(ctx: &mut dyn AppContext, args: CommandArgs) -> CommandResult {
    // Request user confirmation
    if let Some(ailoop_ctx) = ctx.as_any().downcast_ref::<dyn cli_framework::ailoop::AiloopContext>() {
        let confirmed = ailoop_ctx.ailoop_client()
            .request_confirmation(
                "Delete all user data?",
                Some("This action cannot be undone")
            )
            .await?;

        if !confirmed {
            println!("Operation cancelled by user");
            return Ok(());
        }
    }

    // Proceed with dangerous operation
    println!("Deleting all user data...");
    Ok(())
}
```

### Rich Output Formatting

Use CLI output utilities for consistent formatting:

```rust
use cli_framework::cli_output;

// Tables
let data = vec![
    vec!["Service".to_string(), "Status".to_string()],
    vec!["Web".to_string(), "Running".to_string()],
    vec!["DB".to_string(), "Healthy".to_string()],
];
cli_output::print_table(&data)?;

// JSON output
let status = serde_json::json!({
    "services": ["web", "db"],
    "healthy": true
});
cli_output::print_json(&status)?;
```

## Examples

Run the included examples to see the framework in action:

- `cargo run --example basic_cli` - Minimal CLI application with commands
- `cargo run --example with_ask` - CLI with AI-powered natural language commands
- `cargo run --example with_plugins` - CLI with third-party plugin loading
- `cargo run --example with_ailoop` - CLI with human-in-the-loop confirmations

Each example demonstrates different framework capabilities.

## Environment Variables

### LLM Configuration
- `OPENAI_API_KEY` - OpenAI API key for GPT models
- `ANTHROPIC_API_KEY` - Anthropic API key for Claude models
- `LLM_PROVIDER` - Provider selection ("openai", "anthropic")
- `LLM_MODEL` - Model name (defaults: "gpt-4" for OpenAI, "claude-3-sonnet-20240229" for Anthropic)

### ailoop Configuration
- `AILOOP_CHANNEL` - Channel name for human interactions (default: "cli-framework")
- `AILOOP_SERVER_URL` - ailoop server URL (optional, defaults to localhost)

### CLI Behavior
- `NO_COLOR` - Disable colored output
- `QUIET` - Suppress non-essential output

## Documentation

- [Getting Started Tutorial](docs/getting-started.md) - Step-by-step walkthrough
- [Examples Guide](EXAMPLES.md) - Detailed examples and usage patterns
- [API Documentation](https://docs.rs/cli-framework) - Generated API docs (when published)

## Requirements

- Rust 1.70+ (2021 edition)
- Tokio runtime for async operations
- Optional: OpenAI or Anthropic API key for AI features

## License

Apache-2.0
