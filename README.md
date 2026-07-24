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
| `auth` | no | Generic `TokenProvider` trait + `AuthenticatedHttpClient` + four `auth` subcommands; pair with `cli-framework-oidc` for OIDC flows |
| `observability` | no | `tracing-subscriber` logging foundation (implied by `telemetry`) |
| `telemetry` | no | Built-in OpenTelemetry: auto `cli.command` spans exported over OTLP (implies `observability`) |

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

### Startup banner

On startup, `mcp serve` prints a banner showing the connectable URL (HTTP) or
transport mode (stdio) plus the list of registered MCP tools:

```text
┌─ MCP server running ───────────────────────────────────┐

  URL        http://127.0.0.1:9000/mcp
  transport  http (Streamable HTTP)

  Tools (3)
    • myapp_search       Search the catalog
    • myapp_get_item     Fetch an item by id
    • myapp_create_item  Create a new item

  Press Ctrl-C to stop.
└────────────────────────────────────────────────────────┘
```

The tool list is derived from the actually-registered tools at runtime. For
stdio the banner is written to **stderr** (stdout carries the JSON-RPC stream).
The Unicode box degrades to plain ASCII when output is not a TTY or color is
disabled. Output conventions are respected: `QUIET` suppresses the banner, and
`OUTPUT_FORMAT=json` (or an app's `--quiet` / `--json` global flags) emits a
single machine-readable object instead:

```json
{"event":"mcp_started","url":"http://127.0.0.1:9000/mcp","transport":"http","tools":["myapp_search","myapp_get_item","myapp_create_item"]}
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

## Authentication (`auth` feature)

Enable the `auth` feature to get a generic `TokenProvider` trait, an `AuthenticatedHttpClient` that handles bearer injection and automatic 401-retry, and four auto-registered `auth` subcommands:

```toml
[dependencies]
cli-framework = { version = "0.5", features = ["auth"] }
```

Implement `TokenProvider` (or use `cli-framework-oidc`) and pass it to the builder:

```rust
use cli_framework::auth::{AccessToken, AuthError, TokenProvider};
use cli_framework::prelude::*;
use std::sync::Arc;

struct MyProvider;

#[async_trait::async_trait]
impl TokenProvider for MyProvider {
    async fn token(&self) -> Result<AccessToken, AuthError> {
        // Non-interactive only — refresh or client-credentials grant.
        // Never launch an interactive flow here.
        todo!()
    }
    async fn invalidate(&self) { /* clear cached token */ }
}

let app = AppBuilder::new()
    .with_version("my-app", "1.0.0")
    .with_token_provider(Arc::new(MyProvider))
    .build(ctx)?;
```

Once `with_token_provider` is called, four commands are auto-registered:

| Command | Behavior |
|---------|----------|
| `auth login` | Calls `TokenProvider::login()` — launches an interactive flow (e.g. Device Code) |
| `auth logout` | Calls `TokenProvider::logout()` — clears cached tokens |
| `auth status` | Queries token state; `--json` for machine-readable output; `--no-refresh` to skip a network round-trip |
| `auth token` | Prints the raw bearer token to stdout; exits 1 if not authenticated |

Auth commands are **never** exposed as MCP tools or chat tools regardless of the export policy.

### Auth exit codes

| Code | Meaning | Exit |
|------|---------|------|
| `AUTH001` | `login` / `logout` not supported by this provider | 1 |
| `AUTH002` | Provider-level error during token acquisition or login | 1 |
| `AUTH003` | `auth token` called but not authenticated — no token available | 1 |

### OIDC flows (Keycloak and other OIDC providers)

`cli-framework-oidc` (companion crate) works with any OIDC-compliant provider — Keycloak, Azure AD, etc. — and is split into two independent halves:

- **`client`** — `OidcClient` (a `TokenProvider`) with three grant flows (Device Code, Auth Code + PKCE, Client Credentials), OIDC discovery, and an on-disk token cache (`0600`-permissioned).
- **`server`** — an Axum JWT validation layer (`oidc_validation_layer` + `OidcClaims` extractor) that verifies incoming bearer tokens against the provider's JWKS. It handles signing-key rotation (forced refetch on an unknown `kid`) with single-flight + rate-limit bounds so an attacker-supplied `kid` cannot amplify into a fetch flood (ADR 0070).

A consumer enables only the half it needs. See [`cli-framework-oidc/README.md`](cli-framework-oidc/README.md) and the [auth & OIDC skill reference](skill/references/auth-and-oidc.md).

### Using `AuthenticatedHttpClient`

```rust
use cli_framework::auth::AuthenticatedHttpClient;
use cli_framework::http_retry::RetryableHttpClient;

let client = AuthenticatedHttpClient::new(
    RetryableHttpClient::new(reqwest::Client::new()),
    provider.clone(),
);

// Bearer header is injected automatically; a 401 triggers one invalidate+retry.
let resp = client.get("https://api.example.com/data").await?;
```

## Built-in commands

`cli-framework` auto-registers a small set of built-ins during `AppBuilder::build()`:

- `spec`: exports the command surface as JSON/YAML/Markdown.
- `completion <shell>`: emits a simple top-level subcommand completion stub for `bash`, `zsh`, `fish`, or `powershell` (alias: `pwsh`).
- `auth login`, `auth logout`, `auth status`, `auth token`: registered only when `with_token_provider(...)` is called (requires `auth` feature).

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
- `auth login` / `auth logout` called but provider doesn't support the operation (AUTH001) — exit 1
- `auth token` called but not authenticated (AUTH003) — exit 1

**Exit 1 (runtime error)** covers failures that occur *after* arguments are accepted: agent/IO failures, `doctor` reporting health problems (a successful diagnostic run that *found* errors is a runtime result, not a usage error), provider-level auth errors (AUTH002).

`auth status` always exits 0 — it is a query, not a gated operation.

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
| `with_token_provider(provider)` | Supply a `TokenProvider`; auto-registers four `auth` commands (requires `auth` feature) | disabled |
| `with_telemetry(config)` | Enable OpenTelemetry export for CLI runs using a `TelemetryConfig` (requires `telemetry` feature) | disabled |

## Telemetry (`telemetry`)

Opt in with the `telemetry` feature to export OpenTelemetry traces. Every command
dispatch is automatically wrapped in a `cli.command` span carrying the command
path, invocation surface (`cli` / `chat` / `mcp` / `api`), and argument count —
no handler code required. Handlers can also reach a telemetry handle via
`ctx.telemetry()`.

```rust
use cli_framework::app::AppBuilder;
use cli_framework::telemetry::TelemetryConfig;

let app = AppBuilder::new()
    .with_version("myapp", env!("CARGO_PKG_VERSION"))
    // Reads OTEL_* env vars; export is a no-op until an endpoint is set.
    .with_telemetry(TelemetryConfig::from_env())
    .build(ctx)?;
```

CLI runs export via a synchronous `SimpleSpanProcessor` (lossless for short-lived
processes). Long-running servers should instead configure
`ApiServerBuilder::with_telemetry(config, service_name, service_version)`, which
uses an async `BatchSpanProcessor` and flushes on shutdown.

Configuration is driven by the standard `OTEL_*` environment variables (see the
[Environment Variables](#environment-variables) section) or by building a
`TelemetryConfig` directly. Export only activates when an endpoint is set,
`enabled` is true, and `OTEL_SDK_DISABLED` is not `true`.

### Known limitations (v1)

- **Traces only.** The `ctx.telemetry().counter()/histogram()` handles and the
  auto per-command invocation/duration **metrics are not yet exported** — no
  `MeterProvider` is installed and the OTLP `metrics` feature is not compiled.
  These calls are safe but currently discard their values. Tracked for a
  follow-up; prefer spans and `event()` until metrics land.
- `SpanHandle::set_attr` only records span attributes for keys declared at the
  span's callsite; arbitrary keys are dropped (a `tracing` fieldset constraint).
  `record_error` works and sets the span's OTel status to `Error`.
- Config fields `metrics_enabled`, `logs_enabled`, `record_arg_values`, and
  `arg_value_allowlist` are reserved for future signals and not yet consulted.

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
- `cargo run -p cli-framework-oidc --example keycloak_cli --features client` — real OIDC/Keycloak login (env-configured `OidcClient`, `auth` commands, `whoami` via userinfo)

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

### Telemetry Configuration (`telemetry` feature)

Read by `TelemetryConfig::from_env()`. Export stays a no-op until an endpoint is set.

| Variable | Role |
|---------|------|
| `OTEL_EXPORTER_OTLP_ENDPOINT` | OTLP collector base URL (e.g. `http://localhost:4318`); `/v1/traces` is appended automatically |
| `OTEL_SERVICE_NAME` | Overrides the `service.name` resource attribute (defaults to the app name) |
| `OTEL_EXPORTER_OTLP_PROTOCOL` | OTLP protocol; `http/protobuf` (default) is the only value wired today |
| `OTEL_TRACES_SAMPLER_ARG` | Head-sampling ratio in `[0.0, 1.0]` (default `1.0` keeps everything) |
| `OTEL_SDK_DISABLED` | When `true`, vetoes initialisation even if an endpoint is configured |

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
