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

    // Build the app
    let mut builder = AppBuilder::new();
    builder = builder.register_command(hello_command);

    let mut app = builder.build(MyContext)?;

    // Run the app (parses command line arguments)
    app.run().await?;

    Ok(())
}

struct MyContext;
impl AppContext for MyContext {}
```

## AI Ask Command

Enable natural language command resolution. The `ask` command sends your query to an LLM provider, which resolves it to one of your registered commands, then prompts for confirmation before executing:

```rust
use cli_framework::prelude::*;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Set up LLM provider
    std::env::set_var("OPENAI_API_KEY", "your-api-key");
    std::env::set_var("LLM_PROVIDER", "openai");

    let mut builder = AppBuilder::new()
        .with_llm_from_env()?; // Auto-detects from env vars

    // Register commands (ask command is added automatically)
    builder = builder.register_command(deploy_command);

    let mut app = builder.build(MyContext)?;
    app.run().await?;

    // Users can now type:
    // $ myapp ask deploy the app to production
    // $ myapp ask --query "show status" --yes
    
    Ok(())
}
```

### Query syntax

- `ask <query>` — positional words are joined into a single query
- `ask --query "<query>"` — explicit named query

### Confirmation

After the LLM resolves your query, the command displays the resolved command,
confidence score, and reasoning, then prompts:

```
Execute this command? (y/N):
```

Only `y` or `yes` (case-insensitive) proceeds.

### Non-interactive mode

Use `--yes` to skip the confirmation prompt:

```
$ myapp ask "deploy to production" --yes
```

Or set the `ASK_ASSUME_YES` environment variable to `1` or `true` for CI/scripting:

```
ASK_ASSUME_YES=1 myapp ask "deploy to production"
```

## Core Concepts

### Commands

Commands are executable operations in your CLI application. Each command has metadata for AI resolution:

```rust
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
```

### AppContext

`AppContext` holds your application's state and services:

```rust
struct MyAppContext {
    api_client: reqwest::Client,
    config: AppConfig,
}

impl AppContext for MyAppContext {}
```

### Plugin System

Load third-party commands from manifest files:

```toml
# plugin-registry.toml
[plugins.sample]
name = "Sample Plugin"
manifest_path = "/path/to/plugin.json"
enabled = true
```

### ailoop Integration

Add human-in-the-loop confirmations:

```rust
async fn dangerous_command(ctx: &mut dyn AppContext, args: CommandArgs) -> CommandResult {
    let ailoop = ctx.ailoop_client();
    let confirmed = ailoop.request_confirmation(
        "Delete all user data?",
        Some("This action cannot be undone")
    ).await?;

    if confirmed {
        println!("Deleting...");
    }
    Ok(())
}
```

## Examples

Run the included examples to see the framework in action:

- `cargo run --example basic_cli` - Minimal CLI application with commands
- `cargo run --example with_ask` - CLI with AI-powered natural language commands
- `cargo run --example with_plugins` - CLI with third-party plugin loading
- `cargo run --example with_ailoop` - CLI with human-in-the-loop confirmations

## Environment Variables

### LLM Configuration
- `OPENAI_API_KEY` - OpenAI API key
- `ANTHROPIC_API_KEY` - Anthropic API key
- `LLM_PROVIDER` - Provider selection ("openai", "anthropic")
- `LLM_MODEL` - Model name
- `ASK_ASSUME_YES` - Skip confirmation prompt for ask command ("1" or "true")

### ailoop Configuration
- `AILOOP_CHANNEL` - Channel name (default: "cli-framework")
- `AILOOP_SERVER_URL` - ailoop server URL

## License

Apache-2.0 or MIT
