# Getting Started with CLI Framework

This tutorial will guide you through creating your first CLI application using the CLI Framework. By the end, you'll have a working application that can greet users and respond to natural language commands.

## Prerequisites

- Rust toolchain (latest stable, minimum 1.70+)
- Basic familiarity with Rust

## Step 1: Create a New Project

Create a new Rust project:

```bash
cargo new my-cli-app
cd my-cli-app
```

## Step 2: Add Dependencies

Add the following to your `Cargo.toml`:

```toml
[dependencies]
cli-framework = { path = "../cli-framework" } # Adjust path as needed
anyhow = "1.0"
tokio = { version = "1", features = ["full"] }
```

## Step 3: Define Your Application Context

Create `src/main.rs` and start by defining your application context. This is where you'll store application-wide state.

```rust
use cli_framework::prelude::*;

struct MyAppContext {
    app_name: String,
}

impl AppContext for MyAppContext {}
```

## Step 4: Create a Command

Now create a command that greets the user:

```rust
async fn hello_command(ctx: &mut dyn AppContext, args: CommandArgs) -> CommandResult {
    let name = args.positional.get(0).unwrap_or(&"World".to_string()).clone();
    println!("Hello, {}!", name);
    Ok(())
}
```

## Step 5: Build and Run the App

Put it all together in `main()`:

```rust
#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let ctx = MyAppContext {
        app_name: "My CLI App".to_string(),
    };

    // Build the application
    let mut builder = AppBuilder::new();
    
    // Register the command
    builder = builder.register_command(Command {
        id: "hello",
        summary: "Say hello to someone",
        syntax: Some("hello [name]"),
        category: Some("utilities"),
        execute: hello_command,
    });
    
    // Build and run
    let mut app = builder.build(ctx)?;
    app.run().await?;
    
    Ok(())
}
```

## Step 6: Test Your Application

Now you can run your application and pass commands to it:

```bash
# Show help
cargo run

# Run the hello command
cargo run -- hello Alice
```

## Step 7: AI Ask Command

To enable natural language command resolution, configure an LLM provider.

### Setup

```rust
#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Set environment variables (or set them in your shell)
    std::env::set_var("OPENAI_API_KEY", "your-api-key");
    std::env::set_var("LLM_PROVIDER", "openai");

    let ctx = MyAppContext { app_name: "AI CLI".to_string() };

    let mut builder = AppBuilder::new()
        .with_llm_from_env()? // This enables the 'ask' command
        .register_command(hello_command);
    
    let mut app = builder.build(ctx)?;
    app.run().await?;
    
    Ok(())
}
```

### Usage

The `ask` command resolves natural language queries to registered commands:

```bash
# Positional query
cargo run -- ask say hello to Bob

# Named query
cargo run -- ask --query "say hello to Bob"

# Skip confirmation (for CI/scripts)
cargo run -- ask "say hello to Bob" --yes
```

### Confirmation

After resolution, the command displays the result and prompts for confirmation:

```
🎯 Resolved to command:
   Command: hello
   Confidence: 95.0%
   Reasoning: The user wants to greet Bob

Execute this command? (y/N):
```

### Non-interactive mode

Set `ASK_ASSUME_YES=1` in the environment to skip the confirmation prompt:

```bash
ASK_ASSUME_YES=1 cargo run -- ask "say hello to Bob"
```

### Environment Variables

| Variable | Purpose |
|----------|---------|
| `OPENAI_API_KEY` | API key for OpenAI provider |
| `ANTHROPIC_API_KEY` | API key for Anthropic provider |
| `LLM_PROVIDER` | Provider to use (`openai` or `anthropic`) |
| `LLM_MODEL` | Model name (default: `gpt-4` or `claude-3-sonnet`) |
| `ASK_ASSUME_YES` | Set to `1` or `true` to skip confirmation |

## Security Defaults

### Output Sanitization

All strings from LLM responses, plugin data, and external APIs are sanitized before display. ANSI CSI/OSC escape sequences and terminal control characters are stripped automatically. No action is required — sanitization happens at every print site.

### Risk Gate Model

The `ask` command classifies every resolved command into a risk tier before execution:

- **Safe** (default): proceeds normally; `--yes` / `ASK_ASSUME_YES` are honored.
- **Sensitive** (categories: `data`, `config`): requires interactive confirmation when not in `--yes` / CI mode.
- **Destructive** (categories: `deployment`, `admin`, `destructive`): `--yes` and `ASK_ASSUME_YES` are **ignored**. Requires `ALLOW_DESTRUCTIVE_COMMANDS=1` in the environment **and** interactive `y/yes` input.

Override per-command tiers with `AppBuilder::with_risk_policy()`.

### Plugin Path Confinement

Plugin manifest paths are canonicalized and validated to reside under the plugin registry directory. Paths using `../` that would escape the plugin root are rejected with a `PLUGIN_PATH_ESCAPE` error. This prevents path-traversal attacks via crafted registry TOML files.

### Secure HTTP Client

The `secure_reqwest_client()` factory provides a `reqwest::Client` with hardened defaults:

```rust
use cli_framework::http_retry::secure_reqwest_client;

let client = secure_reqwest_client()?; // 5s connect, 30s total, TLS on
```

The existing `RetryableHttpClient::new(client)` constructor is unchanged.

## Next Steps

- Explore [Rich CLI Output](https://github.com/your-org/cli-framework#rich-cli-output) for tables and JSON.
- Add [ailoop integration](https://github.com/your-org/cli-framework#ailoop-integration) for human-in-the-loop confirmations.
- Create [Plugins](https://github.com/your-org/cli-framework#plugin-system) to extend your CLI.
