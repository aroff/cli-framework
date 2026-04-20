# CLI Creation Scenarios (cli-framework)

This reference contains practical scenario templates for organizing commands, subcommands, and mandatory operational commands with `cli-framework`.

## Shared Quick Start (5 minutes)

Use this bootstrap for any scenario:

1. Create project and dependencies.
2. Create command modules by domain.
3. Register minimum mandatory commands first.
4. Add scenario-specific command groups.

```bash
cargo new mytool
cd mytool
```

`Cargo.toml`:

```toml
[dependencies]
cli-framework = { path = "../cli-framework" }
anyhow = "1"
tokio = { version = "1", features = ["full"] }
```

`src/main.rs` (minimal runnable baseline):

```rust
use cli_framework::prelude::*;

struct AppCtx;
impl AppContext for AppCtx {}

async fn health(_ctx: &mut dyn AppContext, _args: CommandArgs) -> CommandResult {
    println!("ok");
    Ok(())
}

async fn version(_ctx: &mut dyn AppContext, _args: CommandArgs) -> CommandResult {
    println!("{}", env!("CARGO_PKG_VERSION"));
    Ok(())
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let mut app = AppBuilder::new()
        .register_command(Command {
            id: "health",
            summary: "Health check",
            syntax: Some("health"),
            category: Some("ops"),
            execute: health,
        })
        .register_command(Command {
            id: "version",
            summary: "Show version",
            syntax: Some("version"),
            category: Some("ops"),
            execute: version,
        })
        .build(AppCtx)?;

    app.run().await
}
```

## Scenario 1: Internal Developer Tool

Goal: automate project bootstrapping and local operations.

Recommended tree:

```text
mytool
  project init
  project list
  project clean
  ops health
  ops doctor
  config show
  version
```

Design notes:
- Keep `project` domain for lifecycle actions.
- Keep `ops` domain for diagnostics only.
- Keep `clean` explicit and destructive by intent.

Quick start implementation:
- Create `src/commands/project/{init.rs,list.rs,clean.rs}`.
- Create `src/commands/ops/{health.rs,doctor.rs,version.rs}`.
- Register IDs as `project.init`, `project.list`, `project.clean`, `ops.health`, `ops.doctor`, `ops.version`.
- Add explicit confirmation in `project clean` before destructive execution.

## Scenario 2: API Client CLI

Goal: expose a remote platform API as stable user commands.

Recommended tree:

```text
platform
  auth login
  auth logout
  auth whoami
  env list
  env use
  deploy create
  deploy list
  deploy status
  ops health
  ops doctor
  config show
  version
```

Design notes:
- Keep authentication isolated under `auth`.
- Keep `deploy` commands verb-first and explicit.
- Keep `env use` as stateful context switch with clear output.

Quick start implementation:
- Add `ApiClient` to `AppContext`.
- Create `auth`, `env`, and `deploy` command modules.
- Implement `auth login/logout/whoami` first, then `deploy` commands.
- Add `ops health` command that checks token presence and API reachability.

Example context shape:

```rust
struct AppCtx {
    api: reqwest::Client,
    base_url: String,
    token: Option<String>,
}

impl AppContext for AppCtx {}
```

## Scenario 3: Data Operations CLI

Goal: run controlled data jobs with safe execution defaults.

Recommended tree:

```text
datactl
  dataset list
  dataset inspect
  job run
  job status
  job cancel
  export create
  export list
  ops health
  ops doctor
  config show
  version
```

Design notes:
- Keep `job run` with dry-run support.
- Keep cancellation explicit and guarded.
- Keep `inspect` read-only and deterministic.

Quick start implementation:
- Create modules: `dataset`, `job`, `export`, `ops`.
- Make `job run` accept `--dry-run` and print execution plan.
- Require explicit `job cancel --id <job-id>` with confirmation.
- Keep `dataset inspect` pure read-only and script-friendly.

## Scenario 4: Multi-Team Platform CLI with Plugins

Goal: provide core stable commands while allowing extension points.

Recommended tree:

```text
corecli
  auth login
  auth logout
  project list
  project use
  plugin list
  plugin enable
  plugin disable
  ops health
  ops doctor
  config show
  version
```

Design notes:
- Keep core security-sensitive operations in built-in commands.
- Keep plugin management explicit and observable.
- Treat plugin errors as isolated failures where possible.

Quick start implementation:
- Implement core commands first (`auth`, `project`, `ops`, `config`).
- Add plugin commands only after core flows are stable.
- Wire plugin registry loading from a manifest file.
- Keep plugin command failures isolated from core command execution.

## Mandatory Command Policy

Use this baseline in all non-trivial CLIs:
- `help` (built-in)
- `version`
- `ops health` or `health`
- `ops doctor` or `doctor`
- `config show`

Add when relevant:
- `auth login`, `auth logout`
- `completion`
- `ask` (only when LLM provider is configured)

## Command Quality Checklist

Use this checklist for each command:
- One command ID maps to one action.
- Side effects are explicit in the command verb.
- Arguments are validated at the command boundary.
- Error messages include direct next-step hints.
- Output is stable and supports scripting mode when required.

## Fast Validation Loop

Run this loop during implementation:

```bash
cargo fmt
cargo clippy -- -D warnings
cargo test
cargo run -- --help
cargo run -- health
cargo run -- version
```

