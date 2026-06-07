# CLI Framework

[![Repository](https://img.shields.io/badge/GitHub-aroff%2Fcli--framework-informational)](https://github.com/aroff/cli-framework)

A Rust library for building CLIs with optional AI-assisted command resolution (**chat**), a plugin registry, ailoop-backed human-in-the-loop prompts, structured command metadata, and async-first dispatch on Tokio.

## Features

- **Chat**: Multi-turn agentic command resolution via `aikit-agent` (default feature)
- **Plugins**: Manifest-driven third-party commands with path validation
- **Human-in-the-loop**: ailoop-core for confirmations
- **Command registry**: Central registration, optional typed `CommandSpec`, and grouping metadata
- **CLI output helpers**: Tables, JSON, progress (behind Cargo features where applicable)
- **Security defaults**: Output sanitization, risk tiers, hardened HTTP helpers
- **MCP Server Mode**: Expose registered commands as MCP tools over Streamable HTTP or stdio (opt-in via `mcp-server` feature)
- **API Server**: Built-in Axum host for serving versioned HTTP APIs with `/healthz` + `/readyz` (opt-in via `api-server` feature)
- **Project Config**: Project root discovery and TOML loading (opt-in via `project-config` feature)

## Cargo Features

| Feature | Default | Description |
|---------|---------|-------------|
| `clap-dispatch` | yes | Clap-based CLI dispatch (no-op since v0.4.0; remove in v0.5.0) |
| `chat` | yes | Multi-turn agentic command resolution via `aikit-agent` |
| `table-advanced` | no | Rich table output via `comfy-table` |
| `progress` | no | Progress bars via `indicatif` |
| `testkit` | no | `CliTestHarness` for in-process testing |
| `mcp-server` | no | Expose commands as MCP tools over HTTP or stdio |
| `api-server` | no | Serve versioned Axum APIs with health/readiness and graceful shutdown |
| `api-swagger` | no | Runtime OpenAPI spec endpoint + embedded Swagger UI (requires `api-server`) |
| `doctor` | no | Structured health-check framework with terminal/JSON output |
| `project-config` | no | Project root discovery and TOML loading (`PC001`–`PC005` error codes) |

## MCP Server Mode

Any binary built with `cli-framework` can become a first-class [Model Context Protocol](https://modelcontextprotocol.io/) server with zero per-command implementation work. LLM agents (Cursor, Claude Desktop, custom agents) can enumerate and invoke all registered commands through the standard MCP protocol.

### Enabling MCP mode

Add the `mcp-server` feature in your binary's `Cargo.toml`:

```toml
[dependencies]
cli-framework = { version = "0.4", features = ["mcp-server"] }
```

### Running

When built with `mcp-server`, `cli-framework` auto-registers an `mcp` command group that includes `mcp serve`:

```bash
# Streamable HTTP (default transport)
my-app mcp serve --port 9000 --path /mcp

# stdio transport (stdin/stdout JSON-RPC)
my-app mcp serve --transport stdio
```

### Tool naming convention

Each registered command is exported as `<app_name>_<command_id>`. Hierarchical commands (e.g. `cluster/get`) use underscores: `myapp_cluster_get`. Underscores (rather than dots) keep tool names within OpenAI's `^[a-zA-Z0-9_-]+$` constraint.

### Schema inference

Each tool's `inputSchema` is derived from `CommandSpec.args`:

| `ArgSpec.value_type` | JSON Schema type |
|---|---|
| `Bool` | `"boolean"` |
| `String` | `"string"` |
| `Int` | `"integer"` |
| `Float` | `"number"` |
| `Enum(variants)` | `{ "type": "string", "enum": [...] }` |
| Repeated option | `{ "type": "array", "items": { "type": "string" } }` |
| Repeated flag (Count) | `"integer"` |

Commands without a `CommandSpec` use a permissive schema `{ "type": "object", "additionalProperties": true }`.

### Cursor integration example

```json
{
  "mcpServers": {
    "my-app": {
      "url": "http://127.0.0.1:8080/mcp"
    }
  }
}
```

### Security

All MCP tool calls are routed through the same validation pipeline as CLI calls: `SpecValidator`, custom validators, and risk policy checks all apply.

- **HTTP MCP**: transport-level authentication/authorization is the operator's responsibility (the MCP endpoint has no built-in auth).
- **stdio MCP**: assumes **local trust** (a local process can spawn and fully control the server). There is no transport auth.
- **stdio stdout constraint**: in stdio mode, **stdout is reserved for JSON-RPC**. Commands and hosts MUST NOT write to stdout (use stderr or structured logging). Writing to stdout will corrupt the MCP transport.
- **Destructive commands**: `ALLOW_DESTRUCTIVE_COMMANDS` and interactive confirmations apply to `chat`; MCP tool calls do not prompt. If you need allowlisting/confirmation for MCP, configure an MCP tool gate via `AppBuilder::with_mcp_tool_gate(...)`.

Choose this crate when you want one stack for classical subcommands plus optional LLM resolution and scripted workflows, without assembling parsing, sanitization, and policy from scratch.

## API Server (`api-server`)

`api-server` provides a framework-owned Axum host for serving your application's HTTP API with a fixed URL shape:

- Versioned APIs live under `/api/{version}/...` (at least one version is required)
- Health endpoints are always present at `/healthz` and `/readyz`; `/healthz` reports a `version` (override it for your app via `health_version(...)`, defaults to the framework's crate version)
- Versioned responses include `X-API-Version: {version}`
- Serve a SPA or static assets at the root via `root_fallback(router)` — framework routes always take priority

When `api-server` is enabled, `cli-framework` re-exports Axum as `cli_framework::axum` so consumers can depend on the exact `axum` version linked by the framework.

### Swagger UI / OpenAPI docs (`api-swagger`)

Enable the `api-swagger` feature to get a runtime OpenAPI spec endpoint and an embedded Swagger UI at no CDN cost:

```toml
[dependencies]
cli-framework = { git = "...", features = ["api-server", "api-swagger"] }
```

Attach your OpenAPI document to each version via the `openapi` field:

```rust
ApiVersion {
    name: ApiVersionName::parse("v1")?,
    router: my_v1_router,
    stability: Stability::Stable,
    deprecation: None,
    #[cfg(feature = "api-swagger")]
    openapi: Some(serde_json::json!({ "openapi": "3.0.3", ... })),
}
```

The framework then serves:

| Path | What it does |
|------|-------------|
| `GET /api/{version}/openapi.json` | App-supplied document with `servers:` patched to `[{"url":"/api/{version}"}]` |
| `GET /api/docs` | Fully embedded Swagger UI (no CDN) with a version switcher |

Versions that set `openapi: None` get no spec endpoint and do not appear in the switcher. Auth gating follows the same `ApiServerBuilder::auth(...)` layer applied to all `/api/**` routes.

## Built-in commands

`cli-framework` auto-registers a small set of built-ins during `AppBuilder::build()`:

- `spec`: exports the command surface as JSON/YAML/Markdown.
- `completion <shell>`: emits a simple top-level subcommand completion stub for `bash`, `zsh`, `fish`, or `powershell` (alias: `pwsh`).

If your app already defines a root-level `completion` command, call `AppBuilder::without_completion()` to opt out of auto-registration and avoid a registration collision.

## Exit-code contract

`App::run()` enforces a two-tier exit-code contract. Consumers can rely on this in CI scripts (`set -e`, `if`-chains, etc.):

| Outcome | Exit code |
|---------|-----------|
| Success | **0** |
| Usage / parse error | **2** |
| Runtime error | **1** |

**Exit 2 (usage error)** covers any error where the user supplied invalid or missing input before the command handler ran:

- Unrecognized subcommand (E001) — `hint:` output is `"Did you mean '<x>'?"` when clap identifies a near match; falls back to `"Use --help to see available commands"` otherwise
- Unknown flag (E002) — `hint:` output is `"Did you mean '--<flag>'?"` when clap identifies a near match; falls back to `"Use --help to see available arguments"` otherwise
- Nested subcommand not found (E012) — `hint:` output is `"Did you mean '<x>'?"` when clap identifies a near match; falls back to `"Use --help to see available commands"` otherwise
- Missing required argument (E003)
- Invalid value type or out-of-set Enum value (E004)
- Conflicting arguments (E005)
- Unsatisfied `requires` constraint (E006)
- Unsupported `completion` shell (E013)
- Unknown `spec --format` value (CS001)
- Unknown `doctor --check` id (DR003)

**Exit 1 (runtime error)** covers failures that occur *after* arguments are accepted: agent/IO failures, `doctor` reporting health problems (a successful diagnostic run that *found* errors is a runtime result, not a usage error).

These errors are signalled to the caller as `Err(UsageError)` from `App::run_with_args()` so test code can inspect the type directly:

```rust
use cli_framework::UsageError;

let result = app.run_with_args(args).await;
if let Err(e) = result {
    if e.downcast_ref::<UsageError>().is_some() {
        // parse/usage error — would have been exit 2 in a real binary
    } else {
        // runtime error — would have been exit 1
    }
}
```

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

## Version output

The framework provides a built-in `version` subcommand and Clap `--version` / `-V`.

- Default output: `{name} {semver}` (e.g. `myapp 1.2.3`)
- Opt-in build id: `{name} {semver} ({short_sha})` (e.g. `myapp 1.2.3 (abc1234)`)

Opt-in without runtime git I/O (compile-time env var):

```rust
use cli_framework::app::AppBuilder;

let app = AppBuilder::new()
    .with_version(env!("CARGO_PKG_NAME"), env!("CARGO_PKG_VERSION"))
    .with_git_sha_short(option_env!("VERGEN_GIT_SHA"))
    .build(ctx)?;
```

One explicit way to provide `VERGEN_GIT_SHA` at build time (consumer crate):

`Cargo.toml`:

```toml
[build-dependencies]
vergen = "8"
```

`build.rs`:

```rust
fn main() {
    // Populate a compile-time env var with the current commit short SHA.
    // This runs at build time (not runtime). Consumers may use `vergen` or any other mechanism.
    let output = std::process::Command::new("git")
        .args(["rev-parse", "--short", "HEAD"])
        .output();

    if let Ok(out) = output {
        if out.status.success() {
            if let Ok(sha) = String::from_utf8(out.stdout) {
                let sha = sha.trim();
                if !sha.is_empty() {
                    println!("cargo:rustc-env=VERGEN_GIT_SHA={sha}");
                }
            }
        }
    }
}
```

## AppBuilder method reference

| Method | Description | Default |
|--------|-------------|---------|
| `register_command(cmd)` | Register a command in the command registry | — |
| `with_version(name, version)` | Enable the built-in `version` subcommand and `--version` flag | disabled |
| `with_git_sha_short(sha)` | Append a short git SHA to version output | `None` |
| `without_completion()` | Opt out of auto-registered `completion` subcommand | enabled |
| `suggest_corrections(bool)` | Enable or disable `"Did you mean?"` suggestions for unknown subcommands and flags (E001, E002, E012). When `true`, the `hint:` line shows the closest match clap identified; when `false`, the generic `"Use --help"` hint is always used. | `true` |
| `with_ailoop_channel(channel)` | Configure the ailoop channel name for HITL interactions | — |
| `with_ailoop_config(config)` | Configure ailoop with a full `AiloopConfig` | — |
| `with_risk_policy(policy)` | Override the default command risk tier policy | — |

## Chat Command (default feature)

`chat` is a default feature providing multi-turn agentic command resolution via `aikit-agent`:

- `cargo build` (default) includes the `chat` command
- Opt out with `default-features = false`

`chat` runs an embedded agent that can call the process's registered commands as tools (tool names and JSON schemas match the MCP export path). Tool-invoked commands execute against the **real AppContext** (no noop dispatch).

`chat` selects mode at runtime:
- One-shot: prompt provided via `--prompt/-p` or stdin is piped
- REPL: no prompt and stdin is a TTY (exits on EOF / Ctrl+C)

LLM configuration is resolved from environment variables used by `aikit-agent` (for example `OPENAI_API_KEY`, `AIKIT_LLM_URL`, `AIKIT_MODEL`), and can be overridden per-run with `--model`.

Notes:
- Tool calls are serialized.
- Ctrl+C cancellation is best-effort; in-flight HTTP requests are cancelled via dropping the request future.
- `--stream` enables server-side streaming, but output is printed once per turn (no structured event stream in this rollout phase).

Try it with the built-in example:

```bash
cargo run --example with_chat -- chat --help
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

> **`[PLANNED]`** — Today the plugin system loads **metadata only**: plugin
> registry + manifest files are parsed and their command descriptions are
> surfaced for discovery. There is no dispatch
> path that actually executes a plugin command (`CommandExecution::Subprocess`
> is declarative only). See [docs/adr/0002-plugins-metadata-only.md](docs/adr/0002-plugins-metadata-only.md).

Declare third-party commands in a manifest file:

```toml
# plugin-registry.toml
[plugins.sample]
name = "Sample Plugin"
manifest_path = "/path/to/plugin.json"
enabled = true
```

### ailoop Integration

ailoop-core provides human-in-the-loop (HITL) interactions for commands that need confirmation, authorization, or notification: all interactions are routed over WebSocket to an `ailoop serve` process. There is no fallback to stdin.

**Pairing requirement:** Start `ailoop serve` before using HITL methods:

```bash
export AILOOP_SERVER=ws://localhost:8080
ailoop serve --port 8080
```

Configure via `AppBuilder::with_ailoop_channel()` or `AppBuilder::with_ailoop_config()`:

```rust
let mut builder = AppBuilder::new()
    .with_ailoop_channel("my-app-channel");
```

Use `cli_framework::ailoop::AiloopClient` inside a command closure for ad-hoc HITL calls:

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

See `skill/examples/with_ailoop` for a full program.

## Examples

Run the included examples to see the framework in action:

- `cargo run --example basic_cli` — Minimal CLI application with commands
- `cargo run --example with_chat` — CLI with AI-backed natural language (`chat`)
- `cargo run --example with_plugins` — CLI with registry-based plugins
- `cargo run --example with_ailoop` — ailoop confirmations and prompts
- `cargo run --example http_retry_demo` — `http_retry` and secure client defaults

Source for each lives under [`skill/examples/`](skill/examples/).

## Security

### Output Sanitization

All strings originating from LLM responses, plugin data, or external APIs are sanitized before display. The sanitizer strips ANSI CSI/OSC escape sequences and terminal control characters, preventing terminal-injection attacks. Printable ASCII, valid UTF-8 multi-byte characters, newlines, tabs, and carriage returns are preserved.

### Command Risk Tiers

The `chat` command classifies every AI-resolved command into one of three risk tiers:

| Tier | Default categories | Behavior |
|---|---|---|
| `Safe` | All others | Proceeds normally |
| `Sensitive` | `data`, `config` | Requires interactive confirmation; ailoop acts as the interactive channel (no TTY needed when ailoop configured) |
| `Destructive` | `deployment`, `admin`, `destructive` | Blocked unless `ALLOW_DESTRUCTIVE_COMMANDS=1`; when set, requires TTY or ailoop for confirmation |

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

Setting `ALLOW_DESTRUCTIVE_COMMANDS=1` permits destructive-tier commands to proceed when combined with interactive confirmation. When ailoop is configured, the ailoop channel acts as the interactive channel (no TTY required). Without ailoop, an interactive terminal is always required.

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

### Chat / LLM Configuration

| Variable | Role |
|---------|------|
| `OPENAI_API_KEY` | API key for the LLM endpoint |
| `AIKIT_LLM_URL` | OpenAI-compatible endpoint URL |
| `AIKIT_MODEL` | Model name override |

### ailoop Configuration

| Variable | Role |
|---------|------|
| `AILOOP_CHANNEL` | Channel name (default: `"cli-framework"`) |
| `AILOOP_SERVER` | WebSocket URL of the paired `ailoop serve` process (default: `ws://localhost:8080`); `http://` and `https://` URLs are normalized to `ws://`/`wss://` automatically |

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
