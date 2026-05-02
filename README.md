# CLI Framework

[![Repository](https://img.shields.io/badge/GitHub-aroff%2Fcli--framework-informational)](https://github.com/aroff/cli-framework)

A Rust library for building CLIs with optional AI-assisted command resolution (**ask**), a plugin registry, ailoop-backed human-in-the-loop prompts, structured command metadata, and async-first dispatch on Tokio.

## Features

- **AI Ask**: Natural language routing to registered commands via OpenAI or Anthropic
- **Plugins**: Manifest-driven third-party commands with path validation
- **Human-in-the-loop**: ailoop-core for confirmations beyond the ask flow
- **Command registry**: Central registration, optional typed `CommandSpec`, and grouping metadata
- **CLI output helpers**: Tables, JSON, progress (behind Cargo features where applicable)
- **Security defaults**: Output sanitization, risk tiers for ask, hardened HTTP helpers

Choose this crate when you want one stack for classical subcommands plus optional LLM resolution and scripted workflows, without assembling parsing, sanitization, and policy from scratch.

## Documentation

| Document | What it covers |
|-----------|----------------|
| [docs/migration-typed-spec.md](docs/migration-typed-spec.md) | How to move from “no **`CommandSpec`**” style code to typed args and stricter flags (**not** deprecated; optional upgrade path) |
| [docs/testing.md](docs/testing.md) | **Automated tests** you write with **`cargo test`**: in-process harness **`CliTestHarness`** (feature **`testkit`**) instead of spawning subprocesses |
| [CONTRIBUTING.md](CONTRIBUTING.md) | Contribute, CI, **system design** (`src/` modules, flow, deps) |

## Quick start

**Prerequisites:** Rust stable (edition **2021**; MSRV is typically **1.70+**) and familiarity with **`async`/Tokio.**

**Create a binary crate** (adjust the path vs `cli-framework` to match your layout):

```bash
cargo new my-cli-app && cd my-cli-app
```

**Dependencies** in `Cargo.toml` (published crate, git, or `path`):

```toml
[dependencies]
cli-framework = { git = "https://github.com/aroff/cli-framework" }
# cli-framework = { path = "../cli-framework" }
anyhow = "1.0"
tokio = { version = "1", features = ["full"] }
```

**Minimal application:** use **`Arc`** for **`execute`**; **`spec`** / **`validator`** are **`None`** until you adopt **`CommandSpec`** (see migration doc).

```rust
use cli_framework::prelude::*;
use std::sync::Arc;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let hello = Command {
        id: "hello",
        summary: "Print a greeting",
        syntax: Some("hello [name]"),
        category: Some("utilities"),
        spec: None,
        validator: None,
        execute: Arc::new(|_ctx, args| {
            Box::pin(async move {
                let name = args
                    .positional
                    .first()
                    .map(String::as_str)
                    .unwrap_or("World");
                println!("Hello, {}!", name);
                Ok(())
            })
        }),
    };

    let mut builder = AppBuilder::new();
    builder = builder.register_command(hello)?;

    let mut app = builder.build(MyContext)?;
    app.run().await?;

    Ok(())
}

struct MyContext;
impl AppContext for MyContext {}
```

**Sanity checks:**

```bash
cargo run
cargo run -- hello Alice
```

## AI Ask Command

Natural-language **`ask`** is registered when an LLM is configured (**`with_llm_from_env()`** or **`with_llm_provider`** after you build the **`AppBuilder`**). Example with one registered command plus **`ask`**:

```rust
use cli_framework::prelude::*;
use std::sync::Arc;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    std::env::set_var("OPENAI_API_KEY", "your-api-key"); // Or set in the shell / use Anthropic vars
    std::env::set_var("LLM_PROVIDER", "openai");

    let hello = Command {
        id: "hello",
        summary: "Say hello",
        syntax: Some("hello [name]"),
        category: Some("utilities"),
        spec: None,
        validator: None,
        execute: Arc::new(|_ctx, args| {
            Box::pin(async move {
                let name = args
                    .positional
                    .first()
                    .map(String::as_str)
                    .unwrap_or("World");
                println!("Hello, {}!", name);
                Ok(())
            })
        }),
    };

    let mut builder = AppBuilder::new().with_llm_from_env()?;
    builder = builder.register_command(hello)?;

    let mut app = builder.build(MyContext)?;
    app.run().await?;

    Ok(())
}

struct MyContext;
impl AppContext for MyContext {}
```

### Query syntax

- `ask <query>` — positional words are joined into a single query
- `ask --query "<query>"` — explicit named query

### Confirmation

After the LLM resolves your query, the command displays the resolved command,
confidence score, and reasoning, then prompts:

```
Resolved to command:
   Command: hello
   Confidence: 95%
   ...

Execute this command? (y/N):
```

Exact formatting may vary slightly by version; only **`y`** or **`yes`** (case-insensitive) proceeds when confirmation is shown.

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
use std::sync::Arc;

let deploy_command = Command {
    id: "deploy",
    summary: "Deploy application to specified environment",
    syntax: Some("deploy --env <environment> --version <version>"),
    category: Some("deployment"),
    spec: None,
    validator: None,
    execute: Arc::new(|_ctx, args| {
        Box::pin(async move {
            let env = args.named.get("env").map(String::as_str).unwrap_or("dev");
            println!("Deploying to {}...", env);
            Ok(())
        })
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

Use `cli_framework::ailoop::AiloopClient` inside a command closure (configure the channel via `AppBuilder::with_ailoop_channel` or environment variables as needed):

```rust
use cli_framework::ailoop::AiloopClient;
// Inside execute:
let ailoop = AiloopClient::new()?;
let confirmed = ailoop
    .request_confirmation("Delete all user data?", Some("This action cannot be undone"))
    .await?;
if confirmed {
    println!("Deleting...");
}
```

See `examples/with_ailoop` for a full program.

## Examples

Run the included examples to see the framework in action:

- `cargo run --example basic_cli` — Minimal CLI application with commands
- `cargo run --example with_ask` — CLI with AI-backed natural language (`ask`)
- `cargo run --example with_plugins` — CLI with registry-based plugins
- `cargo run --example with_ailoop` — ailoop confirmations and prompts
- `cargo run --example http_retry_demo` — `http_retry` and secure client defaults

Source for each lives under [`examples/`](examples/).

## Security

### Output Sanitization

All strings originating from LLM responses, plugin data, or external APIs are sanitized before display. The sanitizer strips ANSI CSI/OSC escape sequences and terminal control characters, preventing terminal-injection attacks. Printable ASCII, valid UTF-8 multi-byte characters, newlines, tabs, and carriage returns are preserved.

### Command Risk Tiers

The `ask` command classifies every AI-resolved command into one of three risk tiers:

| Tier | Default categories | Behavior |
|---|---|---|
| `Safe` | All others | Proceeds normally; `ASK_ASSUME_YES`/`--yes` honored |
| `Sensitive` | `data`, `config` | Requires interactive confirmation in non-interactive mode |
| `Destructive` | `deployment`, `admin`, `destructive` | Blocked unless `ALLOW_DESTRUCTIVE_COMMANDS=1` AND interactive `y/yes` confirmation |

Configure a custom policy via `AppBuilder::with_risk_policy()`:

```rust
use cli_framework::security::command_risk::{CommandRiskPolicy, CommandRiskTier};

let mut policy = CommandRiskPolicy::default();
policy.tiers.insert("my-safe-deploy".to_string(), CommandRiskTier::Safe);

let app = AppBuilder::new()
    .with_risk_policy(policy)
    .build(ctx)?;
```

### `ALLOW_DESTRUCTIVE_COMMANDS` Environment Variable

Setting `ALLOW_DESTRUCTIVE_COMMANDS=1` permits destructive-tier commands to proceed **only when combined with interactive `y/yes` confirmation**. This variable alone is insufficient — an interactive terminal is always required. `--yes` and `ASK_ASSUME_YES` are silently ignored for destructive commands.

### Plugin path confinement

Plugin registry entries are constrained so **`manifest_path`** cannot escape the plugin root (canonical paths, rejection of traversal). Malformed configs fail with **`PLUGIN_PATH_ESCAPE`** instead of loading arbitrary filesystem locations.

### Secure HTTP Client

Use `secure_reqwest_client()` to obtain a `reqwest::Client` with secure defaults:

```rust
use cli_framework::http_retry::secure_reqwest_client;

let client = secure_reqwest_client()?;
let retry_client = RetryableHttpClient::new(client);
```

Defaults: 5s connect timeout, 30s total timeout, built-in TLS roots, TLS certificate verification enabled, no `danger_accept_invalid_certs`.

## Environment Variables

### LLM Configuration

| Variable | Role |
|---------|------|
| `OPENAI_API_KEY` | OpenAI API key |
| `ANTHROPIC_API_KEY` | Anthropic API key |
| `LLM_PROVIDER` | **`openai`** or **`anthropic`** |
| `LLM_MODEL` | Override model id (providers pick defaults otherwise) |
| `ASK_ASSUME_YES` | Set **`1`** or **`true`** to skip confirmation for **Safe** / **Sensitive** ask resolutions (destructive tier still gated per policy) |

### ailoop Configuration
- `AILOOP_CHANNEL` - Channel name (default: "cli-framework")
- `AILOOP_SERVER_URL` - ailoop server URL

## Migration Guide

Upgrading to the typed `CommandSpec` model? See [docs/migration-typed-spec.md](docs/migration-typed-spec.md) for step-by-step instructions on:
- Adding `spec: None, validator: None` fields to existing `Command` literals
- Adopting `CommandSpec` for validated argument parsing
- Updating `register_command` call sites (now returns `Result<Self>`)
- Using `CliTestHarness` for in-process test capture (see [docs/testing.md](docs/testing.md))

## License

Apache-2.0

## Contributing

See [CONTRIBUTING.md](CONTRIBUTING.md).
