# CLI creation scenarios (cli-framework)

Practical layouts for **`CommandPath`** registration and operational baselines. All Rust snippets assume `cli-framework` plus `tokio` and `anyhow`.

## Shared bootstrap

```bash
cargo new mytool
cd mytool
```

`Cargo.toml` (adjust source: `git`, `path`, or crates.io when published):

```toml
[dependencies]
cli-framework = { git = "https://github.com/aroff/cli-framework" }
anyhow = "1"
tokio = { version = "1", features = ["full"] }
```

Minimal `main.rs` with **`health`** and **`version`** (note `Arc` execute and `register_command(…)?`):

```rust
use cli_framework::prelude::*;
use std::sync::Arc;

struct AppCtx;
impl AppContext for AppCtx {}

fn health_command() -> Command {
    Command {
        id: "health",
        summary: "Health check",
        syntax: Some("health"),
        category: Some("ops"),
        spec: None,
        validator: None,
        execute: Arc::new(|_ctx, _args| Box::pin(async move {
            println!("ok");
            Ok(())
        })),
    }
}

fn version_command() -> Command {
    Command {
        id: "version",
        summary: "Show version",
        syntax: Some("version"),
        category: Some("ops"),
        spec: None,
        validator: None,
        execute: Arc::new(|_ctx, _args| Box::pin(async move {
            println!("{}", env!("CARGO_PKG_VERSION"));
            Ok(())
        })),
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let mut builder = AppBuilder::new();
    builder = builder
        .register_command(health_command())?
        .register_command(version_command())?;
    let mut app = builder.build(AppCtx)?;
    app.run().await
}
```

**Nested IDs:** register with `builder.register_command_at(&CommandPath::new(&["project", "init"])?, cmd)?` where **`cmd.id`** is typically the leaf (e.g. `"init"`). Do not confuse dotted UI labels with `CommandPath` (paths use `/` in string form: `project/init`).

## Scenario 1: Internal developer tool

Suggested surface:

```text
project/init, project/list, project/clean
ops/health, ops/doctor
config/show
version   (root or ops)
```

Structure modules under `src/commands/` by domain; keep destructive **`project/clean`** behind confirmation and accurate risk/category metadata if using **`ask`**.

## Scenario 2: API client CLI

Suggested surface:

```text
auth/login, auth/logout, auth/whoami
env/list, env/use
deploy/create, deploy/list, deploy/status
ops/health, ops/doctor
config/show
```

Put **`reqwest::Client`** (or `secure_reqwest_client` from `cli_framework::http_retry`) and base URL/token on **`AppContext`**. Implement **`ops/health`** to validate config and reachability early.

## Scenario 3: Data operations CLI

Suggested surface:

```text
dataset/list, dataset/inspect
job/run, job/status, job/cancel
export/create, export/list
ops/*, config/show
```

Prefer **`job/run --dry-run`** style plans where applicable; make cancel/destructive actions explicit.

## Scenario 4: Multi-team platform CLI with plugins

Core built-ins first (**auth**, **project**, **ops**, **config**), then **`PluginRegistryManager`** flows from manifests (see **`README.md`** plugin section). Keep plugin failures from taking down unrelated commands where possible.

## Mandatory command policy (baseline)

Recommended for shipping tools users operate in production:

- Built-in **`--help`**
- **Version** visibility (`with_version` and/or **`version`** command)
- **`health`** or **`ops/health`**
- **`doctor`** / deep diagnostics where complexity warrants
- **`config show`** when configuration is non-trivial

Add when relevant: **`auth login`/`logout`**, shell **completion**, **`ask`** (only with LLM configuration and README security expectations).

## Command quality checklist

- One path → one behavior; verbs name side effects honestly.
- Validate at the boundary; return errors with concrete next steps.
- Script-friendly modes where users automate (stable output, documented exit codes).

## Fast validation loop

```bash
cargo fmt --check
cargo clippy -- -D warnings
cargo test
cargo run -- --help
cargo run -- health
cargo run -- version
```
