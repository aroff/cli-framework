---
name: tools-cli-framework
description: >-
  Guides building and refactoring Rust CLIs with the cli-framework crate (tokio/async
  dispatch, Arc command handlers, CommandSpec, plugins, ailoop, chat, OpenTelemetry).
  Use when the user or codebase mentions cli-framework, AppBuilder, Command registration,
  natural language chat, plugin registry, operational command layout for Rust binaries,
  or telemetry/tracing/metrics/OTLP export on a cli-framework-based service.
license: Apache-2.0
metadata:
  version: "0.4.0"
---

# cli-framework integration

Implement CLIs using [`cli-framework`](https://github.com/aroff/cli-framework): `AppBuilder`, `Command` (with `Arc` async executors), optional `chat`, plugins, and ailoop. Prefer repository docs over duplicating them here.

## 1. When to use

Load this skill when:
- Building a new or refactored Rust CLI on the cli-framework stack
- Registering command trees, filling `CommandSpec` metadata
- Implementing risk-aware `chat` flows, plugin manifests, ailoop confirmations, or HTTP retry usage
- The codebase imports `cli_framework`, references `AppBuilder`, or uses `register_command`

## 2. Dependency baseline

```toml
[dependencies]
cli-framework = { git = "https://github.com/aroff/cli-framework" }
anyhow = "1"
tokio = { version = "1", features = ["full"] }
```

Use `path = "..."` when developing inside a workspace or alongside the crate source.

Enable optional features as needed:

```toml
cli-framework = { git = "...", features = ["mcp-server", "testkit"] }
```

See `skill/references/features-and-cargo-flags.md` for the full feature table.

## 3. Minimal skeleton

`register_command` returns `Result`; `execute` is `Arc<…>`; set `spec` and `validator` to `None` until adopting typed specs.

```rust
use cli_framework::prelude::*;
use std::sync::Arc;

struct AppCtx;
impl AppContext for AppCtx {}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let mut app = AppBuilder::new()
        .register_command(Command {
            id: "health",
            summary: "Quick readiness check",
            syntax: Some("health"),
            category: Some("ops"),
            spec: None,
            validator: None,
            expose_mcp: false,
            execute: Arc::new(|_ctx, _args| Box::pin(async move {
                println!("ok");
                Ok(())
            })),
        })?
        .build(AppCtx)?;
    app.run().await
}
```

## 4. Prelude and `AppContext`

```rust
use cli_framework::prelude::*;
```

This re-exports `AppBuilder`, `App`, `Command`, `CommandArgs`, `AppContext`, `CommandSpec`, `ArgSpec`, `ArgValueType`, `Cardinality`, and `CommandPath`.

Implement `AppContext` on your context struct; it requires no methods by default but can carry shared clients:

```rust
struct AppCtx {
    http: reqwest::Client,
    base_url: String,
}
impl AppContext for AppCtx {}
```

## 5. `AppBuilder` and `Arc` executor pattern

The builder chain is: `AppBuilder::new()` → `.register_command(cmd)?` → `.build(ctx)?` → `app.run().await`.

```rust
let mut builder = AppBuilder::new();
builder = builder
    .register_command(health_cmd())?
    .register_command(version_cmd())?;
let mut app = builder.build(AppCtx { /* ... */ })?;
app.run().await
```

Each `Command.execute` is an `Arc<dyn Fn(Arc<C>, CommandArgs) -> BoxFuture<'static, anyhow::Result<()>> + Send + Sync>`.

## 6. `register_command` / `register_command_at` and `CommandPath`

Flat registration (root-level command):

```rust
builder.register_command(cmd)?;
```

Hierarchical registration:

```rust
builder.register_command_at(&CommandPath::new(&["project", "init"])?, cmd)?;
```

`CommandPath::new` validates segments; string form uses `/` as separator (e.g. `"project/init"`). The `cmd.id` should be the leaf name (e.g. `"init"`).

## 7. `CommandSpec`, `ArgSpec`, `ArgValueType`, `Cardinality`

```rust
use cli_framework::prelude::*;

fn deploy_cmd() -> Command {
    Command {
        id: "create",
        summary: "Create a new deployment",
        syntax: Some("deploy create --env <env> [--dry-run]"),
        category: Some("deploy"),
        spec: Some(CommandSpec {
            args: vec![
                ArgSpec {
                    name: "env",
                    long: Some("env"),
                    help: Some("Target environment"),
                    value_type: ArgValueType::String,
                    cardinality: Cardinality::Required,
                    ..Default::default()
                },
                ArgSpec {
                    name: "dry_run",
                    long: Some("dry-run"),
                    help: Some("Simulate without applying"),
                    value_type: ArgValueType::Bool,
                    cardinality: Cardinality::Optional,
                    ..Default::default()
                },
            ],
        }),
        validator: None,
        execute: Arc::new(|_ctx, args| Box::pin(async move {
            let env = args.get("env").unwrap();
            println!("Deploying to {env}");
            Ok(())
        })),
    }
}
```

`ArgValueType` variants: `String`, `Int`, `Float`, `Bool`.
`Cardinality` variants: `Required`, `Optional`, `Multi`.

## 8. `validator`

Custom validation runs after `SpecValidator` and before `execute`:

```rust
validator: Some(Arc::new(|args| {
    if args.get("env").map(|v| v == "prod").unwrap_or(false) {
        return Err(anyhow::anyhow!("Use deploy/create-prod for production"));
    }
    Ok(())
})),
```

See `skill/references/command-spec-and-validation.md` for full `SpecValidator` details.

## 9. Operational baseline

For any non-trivial tool, plan these commands:

| Command | Purpose |
|---------|---------|
| `--help` / built-in help | Auto-generated from `CommandSpec` |
| `version` | Show crate version; use `AppBuilder::with_version` |
| `health` / `ops/health` | Validate config and reachability |
| `doctor` / `ops/doctor` | Deep diagnostics for support |
| `config show` | Print effective config when non-trivial |
| `auth login` / `auth logout` | When remote APIs are involved |
| `chat` | Natural language command resolution (default feature) |

## 10. Chat command and security tiers

The `chat` command is a default feature providing multi-turn agentic command resolution via `aikit-agent`. Configure via environment:

- `AIKIT_LLM_URL` — OpenAI-compatible endpoint URL
- `AIKIT_MODEL` — model name
- `OPENAI_API_KEY` — API key

Risk tiers applied before `execute`:

| Tier | When | Behavior |
|------|------|---------|
| Safe | Read-only commands | Execute directly |
| Sensitive | Config mutations, auth | Prompt for confirmation |
| Destructive | Data deletion, irreversible ops | Blocked unless `ALLOW_DESTRUCTIVE_COMMANDS=1` |

Set `ALLOW_DESTRUCTIVE_COMMANDS=1` in controlled environments to permit destructive flows. All external output is sanitized before display via `src/security/output_sanitize.rs`.

See `skill/references/cli-creation-scenarios.md` for domain-specific layouts.

## 11. Plugins and ailoop

Plugin manifests are TOML files loaded by `PluginRegistryManager`. Each plugin declares commands that are registered alongside built-ins. See `skill/references/plugins-and-ailoop.md`.

ailoop confirmation pattern:

```rust
use cli_framework::ailoop::AiloopClient;

let ailoop = AiloopClient::new()?;
let confirmed = ailoop
    .request_confirmation("Delete all user data?", Some("This action cannot be undone"))
    .await?;
if confirmed { /* proceed */ }
```

Enable via `AppBuilder::with_ailoop_channel`. See `skill/examples/with_ailoop` and `skill/examples/with_plugins`.

## 12. MCP auto-serve mode

Enable with `features = ["mcp-server"]`. At runtime:

```bash
my-app --mcp-serve [--mcp-host 0.0.0.0] [--mcp-port 9000] [--mcp-path /mcp]
```

All registered commands become MCP tools. Tool names follow `<app_name>.<command_id>` (slashes become dots). The same validation pipeline (SpecValidator → custom validator → risk policy) applies to every MCP call.

Full detail — error codes, concurrency model, Newton port notes — in `skill/references/mcp-streamable-http.md`. See `skill/examples/with_mcp`.

## 13. HTTP retry

```rust
use cli_framework::http_retry::{RetryableHttpClient, secure_reqwest_client};

let client = RetryableHttpClient::builder()
    .max_retries(3)
    .build(secure_reqwest_client()?)?;
```

`secure_reqwest_client` enforces TLS, disables redirects to non-HTTPS, and sets a timeout. Retryable vs. non-retryable error classification is automatic. See `skill/references/http-retry.md` and `skill/examples/http_retry_demo`.

## 14. Telemetry (`telemetry` feature)

Enable with `features = ["telemetry"]`. Every command dispatch — CLI, chat,
MCP, and `version` — gets an automatic `cli.command` OpenTelemetry span plus
`cli.command.invocations`/`cli.command.duration_ms` metrics, no handler code
required:

```rust
use cli_framework::telemetry::TelemetryConfig;

let app = AppBuilder::new()
    .with_version("myapp", env!("CARGO_PKG_VERSION"))
    .with_telemetry(TelemetryConfig::from_env())   // no-op until an OTLP endpoint is set
    .build(ctx)?;
```

**The one trap that matters**: `with_telemetry()` installs the process's
`tracing` subscriber. If anything else installs one first — including this
framework's own `init_default_logging()` — the OTel bridge cannot attach and
export goes silently to nowhere (a warning is printed to stderr, not
`tracing::warn!`, since the subscriber may not be listening). Call
`init_default_logging()` only when telemetry is inactive; never both.

Outbound distributed tracing requires one explicit call per HTTP request —
`with_trace_context()` on a `reqwest::RequestBuilder`
(`cli_framework::telemetry::propagation::TracedRequestBuilder`) — inbound
extraction on `ApiServerBuilder` is automatic. Skip it and every service
downstream produces a disconnected trace instead of joining the caller's.

Full detail — subscriber-ownership trap, propagation, OTLP auth headers,
protocol/signal config, app-level `ctx.telemetry()` usage, testing pattern —
in `skill/references/telemetry.md`.

## 15. Testkit

```rust
// In test, with features = ["testkit"]
use cli_framework::testkit::CliTestHarness;

let harness = CliTestHarness::new(build_app()?);
let output = harness.run(&["health"]).await?;
assert!(output.contains("ok"));
```

The `testkit` feature enables in-process CLI testing without spawning a subprocess. See `skill/references/testing-with-testkit.md` and `docs/testing.md`.

## 16. Doctor diagnostics framework (`doctor` feature)

Enable with `features = ["doctor"]`. Provides a structured `DoctorCheck` trait, concurrent runner, and a `doctor` CLI command with terminal table and JSON output.

### Implementing a check

```rust
use cli_framework::doctor::{CheckSeverity, DoctorCheck, DoctorFinding, DoctorFuture};
use cli_framework::app::AppContext;

struct MyCheck;

impl DoctorCheck for MyCheck {
    fn id(&self) -> &'static str { "my-check" }
    fn title(&self) -> &'static str { "My check" }
    fn run(&self, _ctx: &dyn AppContext) -> DoctorFuture {
        Box::pin(async {
            DoctorFinding {
                check_id: "my-check".to_string(),
                title: "My check".to_string(),
                severity: CheckSeverity::Ok,
                message: "All good".to_string(),
                detail: None,
                remediation: None,
            }
        })
    }
}
```

`CheckSeverity` values: `Ok`, `Warning`, `Error`, `Skipped`. Only `Error` findings trigger exit code 1.

### Registering checks via `DoctorModule`

```rust
use cli_framework::doctor::{DoctorCheck, DoctorModule};
use std::sync::Arc;

let checks: Vec<Arc<dyn DoctorCheck>> = vec![Arc::new(MyCheck)];
let app = AppBuilder::new()
    .register_module(DoctorModule::new(checks))?
    .build(ctx)?;
```

Or directly via `register_doctor_checks`:

```rust
let app = AppBuilder::new()
    .register_doctor_checks(vec![Arc::new(MyCheck)])
    .build(ctx)?;
```

### Built-in checks

```rust
use cli_framework::doctor::builtin::{EnvRequiredCheck, TmpdirWritableCheck};

// Checks HOME and PATH are set and non-empty (Error if missing)
Arc::new(EnvRequiredCheck::new(&["HOME", "PATH"]))

// Checks temp dir is writable (Warning if not)
Arc::new(TmpdirWritableCheck)
```

### CLI usage

```
myapp doctor              # terminal table output
myapp doctor --json       # JSON to stdout
myapp doctor --check <id> # run one check by ID
```

### Exit code policy

- Exit 0: no `Error`-severity findings (warnings and skipped do not affect exit code)
- Exit 1: one or more `Error`-severity findings

CI pipelines can rely on this: `myapp doctor --json | jq '.summary.errors'`.

### Example

```bash
cargo run --example with_doctor --features doctor
```

## 17. Authentication (`auth` feature)

Enable with `features = ["auth"]` in `cli-framework`. Pair with `cli-framework-oidc` (feature `client`) for OIDC flows.

```toml
[dependencies]
cli-framework = { git = "...", features = ["auth"] }
cli-framework-oidc = { path = "../cli-framework-oidc", features = ["client"] }
dirs = "5"
```

Wire an `OidcClient` and the four `auth` commands register automatically:

```rust
use cli_framework_oidc::client::{OidcClient, OidcFlow};
use std::sync::Arc;

let cache_dir = dirs::cache_dir().unwrap().join("my-app");

let oidc = Arc::new(
    OidcClient::builder()
        .issuer_url("https://keycloak.example.com/realms/my-realm")
        .client_id("my-cli")
        .flow(OidcFlow::DeviceCode)   // or AuthCodePkce / ClientCredentials
        .cache_dir(cache_dir)
        .build()?
);

let app = AppBuilder::new()
    .with_version("my-app", "1.0.0")
    .with_token_provider(oidc.clone())   // registers: auth login/logout/status/token
    .build(ctx)?;
```

Commands that call protected APIs use `AuthenticatedHttpClient`:

```rust
use cli_framework::auth::AuthenticatedHttpClient;
use cli_framework::http_retry::RetryableHttpClient;

let api = AuthenticatedHttpClient::new(
    RetryableHttpClient::new(reqwest::Client::new()),
    oidc.clone(),
);
// Bearer header injected automatically; 401 → invalidate + refresh + retry once
let resp = api.get("https://api.example.com/things").await?;
```

Full detail — Keycloak client config, all three flows, cache file schema, runtime caching logic, JWT server validation, testing with stubs — in `skill/references/auth-and-oidc.md`. Runnable example: `skill/examples/with_auth`.

## 18. Pointers to `references/` and `skill/examples/`

### Reference files

| File | Topic |
|------|-------|
| `skill/references/architecture-and-sources.md` | Module map, source locations |
| `skill/references/command-registration-and-context.md` | `AppBuilder`, `AppContext`, dispatch flow |
| `skill/references/command-spec-and-validation.md` | `CommandSpec`, `ArgSpec`, `SpecValidator` |
| `skill/references/features-and-cargo-flags.md` | All features + `cli-framework-oidc`, defaults, TOML snippets |
| `skill/references/auth-and-oidc.md` | `TokenProvider`, `OidcClient`, Keycloak setup, cache schema, `AuthenticatedHttpClient`, JWT server layer, testing |
| `skill/references/mcp-streamable-http.md` | Full MCP reference: flags, error codes, concurrency |
| `skill/references/plugins-and-ailoop.md` | Plugin manifests, ailoop confirmations |
| `skill/references/http-retry.md` | `RetryableHttpClient`, circuit breaker |
| `skill/references/telemetry.md` | OpenTelemetry: subscriber trap, auto spans/metrics, W3C propagation, OTLP auth headers, testing |
| `skill/references/testing-with-testkit.md` | `CliTestHarness`, in-process test pattern |
| `skill/references/cli-creation-scenarios.md` | Domain CLI layouts (internal tooling, API client, data ops, plugins) |

### Runnable examples

| Example | How to run | Feature flags |
|---------|-----------|---------------|
| `skill/examples/basic_cli` | `cargo run --example basic_cli` | none |
| `skill/examples/with_plugins` | `cargo run --example with_plugins` | none |
| `skill/examples/with_ailoop` | `cargo run --example with_ailoop` | none |
| `skill/examples/with_mcp` | `cargo run --example with_mcp --features mcp-server` | `mcp-server` |
| `skill/examples/with_doctor` | `cargo run --example with_doctor --features doctor` | `doctor` |
| `skill/examples/with_auth` | `cargo run --example with_auth --features auth` | `auth` |
| `skill/examples/http_retry_demo` | `cargo run --example http_retry_demo` | none |
