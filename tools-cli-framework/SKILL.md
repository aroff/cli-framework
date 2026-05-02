---
name: tools-cli-framework
description: >-
  Guides building and refactoring Rust CLIs with the cli-framework crate (tokio/async
  dispatch, Arc command handlers, CommandSpec, plugins, ailoop, LLM ask). Use when the
  user or codebase mentions cli-framework, AppBuilder, Command registration, natural
  language ask, plugin registry, or operational command layout for Rust binaries.
license: Apache-2.0 (same as cli-framework crate)
metadata:
  version: "0.2.0"
---

# cli-framework integration

Implement CLIs using [`cli-framework`](https://github.com/aroff/cli-framework): `AppBuilder`, `Command` (with `Arc` async executors), optional `ask`, plugins, and ailoop. Prefer repository docs over duplicating them here.

## When to use

- New or refactored Rust CLI on this stack
- Command trees, metadata for LLM resolution, risk-aware `ask` flows
- Plugin manifests, ailoop confirmations, `http_retry` usage

## Dependency baseline

```toml
[dependencies]
cli-framework = { git = "https://github.com/aroff/cli-framework" }
anyhow = "1"
tokio = { version = "1", features = ["full"] }
```

Use `path = "..."` when developing inside a workspace.

## Minimal skeleton (compiles)

`register_command` returns `Result`; `execute` is `Arc<…>`; set `spec` and `validator` to `None` until adopting typed specs (see repo `docs/migration-typed-spec.md`).

```rust
use cli_framework::prelude::*;
use std::sync::Arc;

struct AppCtx;
impl AppContext for AppCtx {}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let mut builder = AppBuilder::new().register_command(Command {
        id: "health",
        summary: "Quick readiness check",
        syntax: Some("health"),
        category: Some("ops"),
        spec: None,
        validator: None,
        execute: Arc::new(|_ctx, _args| Box::pin(async move {
            println!("ok");
            Ok(())
        })),
    })?;

    if std::env::var("LLM_PROVIDER").is_ok() || std::env::var("OPENAI_API_KEY").is_ok()
    {
        builder = builder.with_llm_from_env()?;
    }

    let mut app = builder.build(AppCtx)?;
    app.run().await
}
```

## Hierarchical commands

- **Path** (CLI resolution): use `register_command_at` with `CommandPath::new(&["project", "init"])` (slashes in string form as `project/init`).
- **`Command.id`**: stable leaf identifier, e.g. `"init"`, must match how you resolve and document the command (see `src/command/registry.rs` tests).

## Command design

- Group by domain in paths or ID prefixes; keep flags as modifiers, not alternate verbs.
- Keep handlers thin; put logic in modules or context-held services.
- For `ask`, fill **`summary`**, **`syntax`**, and **`category`** accurately; destructive flows must respect README security tiers (`ALLOW_DESTRUCTIVE_COMMANDS`, interactive confirmation).

## Operational baseline (recommended)

For non-trivial tools, plan for discovery and ops: `--help`/built-in help, **version** (often `with_version` on builder plus or instead of a `version` command), **health**, **doctor**/**diagnostics**, **config show**. Add **`auth`** when remote APIs matter; **`ask`** only with LLM env configured.

## Implementation order

1. `AppContext` and shared clients
2. Core paths and mandatory/diagnostic commands
3. Domain commands; then plugins if needed
4. Enable `with_llm_from_env()` when `ask` is stable
5. Tests per domain (`cargo test`, feature-gated suites as in repo `Cargo.toml`)

## Repo references (prefer reading these)

| Topic | Location |
|--------|-----------|
| User overview | `README.md` (quick start, ask, security) |
| Typed args | `docs/migration-typed-spec.md` |
| Tests | `docs/testing.md` |
| Layout / design | `CONTRIBUTING.md` |
| Runnable samples | `examples/basic_cli`, `examples/with_ask`, `examples/with_plugins`, `examples/with_ailoop`, `examples/http_retry_demo` |

## Detailed scenarios

See `references/cli-creation-scenarios.md` for domain-oriented command maps (internal tooling, API client, data ops, plugins).
