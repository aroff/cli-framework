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

## Step 7: Adding AI "Ask" Support (Optional)

To enable natural language commands, you just need to configure an LLM provider:

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

Now you can use natural language:

```bash
cargo run -- ask say hello to Bob
```

## Next Steps

- Explore [Rich CLI Output](https://github.com/your-org/cli-framework#rich-cli-output) for tables and JSON.
- Add [ailoop integration](https://github.com/your-org/cli-framework#ailoop-integration) for human-in-the-loop confirmations.
- Create [Plugins](https://github.com/your-org/cli-framework#plugin-system) to extend your CLI.
