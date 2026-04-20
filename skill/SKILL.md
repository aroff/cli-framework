---
name: tools-cli-framework
version: 0.1.0
description: This skill should be used when building or refactoring Rust CLI applications with cli-framework, including command design, subcommand hierarchy, and mandatory operational command sets.
---

# CLI Framework Implementation Guide

Implement and refactor Rust CLI tools with `cli-framework` using a clean command architecture and a required operational command baseline.

## When to Use This Skill

Apply this skill when:
- Creating a new CLI tool on top of `cli-framework`
- Refactoring an existing CLI with weak command boundaries
- Defining command groups, subcommands, and required operational commands
- Enabling `ask`, plugin loading, or ailoop-based confirmations
- Standardizing CLI architecture across multiple projects

## Core Integration

Use this dependency baseline:

```toml
[dependencies]
cli-framework = { path = "../cli-framework" }
anyhow = "1"
tokio = { version = "1", features = ["full"] }
```

Start from this minimal skeleton:

```rust
use cli_framework::prelude::*;

struct AppCtx;
impl AppContext for AppCtx {}

async fn health_command(_ctx: &mut dyn AppContext, _args: CommandArgs) -> CommandResult {
    println!("ok");
    Ok(())
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let mut builder = AppBuilder::new().register_command(Command {
        id: "health",
        summary: "Check service health",
        syntax: Some("health"),
        category: Some("ops"),
        execute: health_command,
    });

    // Enable ask command only when an LLM provider is configured.
    if std::env::var("LLM_PROVIDER").is_ok() {
        builder = builder.with_llm_from_env()?;
    }

    let mut app = builder.build(AppCtx)?;
    app.run().await
}
```

## Command Architecture Rules

Apply these rules:
- Group top-level commands by domain (`project`, `auth`, `ops`, `config`)
- Implement subcommands as actions (`project init`, `project list`)
- Keep flags as modifiers, not alternate actions
- Keep command handlers thin and move logic into services
- Keep command IDs stable across versions

## Mandatory Commands Baseline

Include these commands in every non-trivial CLI:
- `help` (built-in): discoverability and syntax guidance
- `version`: exact app version and build metadata
- `health`: fast local readiness check
- `doctor`: deep diagnostics (config, auth, connectivity, permissions)
- `config show`: current effective configuration

Include these commands when relevant:
- `auth login` and `auth logout` for remote/API-backed tools
- `completion` shell completion generation
- `ask` natural language command resolver (only with LLM config)

## Ask Command Rules

Register high-quality metadata for each command:
- Fill `summary` with outcome-focused language
- Fill `syntax` with real flag placeholders
- Set `category` to the owning domain

Protect execution:
- Resolve ask intent to a concrete command and arguments
- Display a dry-run preview before destructive actions
- Require explicit confirmation for destructive paths

## Implementation Sequence

Follow this sequence:
1. Define command map and mandatory command set
2. Define `AppContext` and shared services
3. Implement commands by domain module
4. Register commands in one bootstrap module
5. Add `ask` only after baseline commands are stable
6. Add unit and integration tests for each command group

## References

Read detailed examples and scenarios:
- `references/cli-creation-scenarios.md`

Use workspace source references:
- `/README.md`
- `/docs/getting-started.md`
- `/examples/basic_cli/src/main.rs`
- `/examples/with_ask/src/main.rs`
