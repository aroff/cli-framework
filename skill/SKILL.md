# cli-framework MCP Auto-Serve Skill

This document describes how to implement and use the MCP server mode in binaries built with `cli-framework`.

## Overview

Enabling the `mcp-server` Cargo feature turns any `cli-framework` binary into an MCP Streamable HTTP server. All registered commands are automatically exported as MCP tools with JSON Schema derived from their `CommandSpec`.

## Enabling MCP mode

```toml
[dependencies]
cli-framework = { version = "0.3", features = ["mcp-server", "clap-dispatch"] }
```

Start the server at runtime:

```bash
my-app --mcp-serve --mcp-host 0.0.0.0 --mcp-port 9000 --mcp-path /mcp
```

## CommandSpec quality for MCP

The quality of MCP tool schemas is directly proportional to the quality of your `CommandSpec` metadata.

**Do:**
- Provide a meaningful `summary` on each `CommandSpec` — it becomes the tool's `description` in `tools/list`.
- Declare all arguments with accurate `ArgValueType` — enables correct JSON Schema generation.
- Mark arguments as `Cardinality::Required` when they are mandatory — they appear in `inputSchema.required`.
- Use `ArgSpec.long` to set a user-facing flag name different from the internal `name`.
- Use `ArgSpec.help` for per-argument descriptions.

**Avoid:**
- Registering commands without a `CommandSpec` if LLM usability matters — they fall back to a permissive `{ "type": "object", "additionalProperties": true }` schema, which provides no type guidance to the agent.
- Using internal identifiers (underscores, abbreviations) as `ArgSpec.name` without setting `long` — the schema property name is `long ?? name`.

## Tool naming convention

```
<app_name>.<command_id>
```

- `app_name` comes from `AppBuilder::with_version(name, _)`.
- Hierarchical commands: `cluster/get` → `myapp.cluster.get` (slashes become dots).
- If `app_name` is `"unknown"`, a startup warning is emitted.

## Validation and safety

MCP tool calls are routed through the **same validation pipeline** as CLI calls:
1. `SpecValidator` checks required args, type conformance, conflicts, and `requires` constraints.
2. Custom command validators (if registered) are run.
3. Risk policy checks (from `CommandRiskPolicy`) apply.

There is no way to bypass validation from an MCP client. This is by design (DD5).

## Error codes

| Code | When |
|---|---|
| `MCP_CMD_NOT_FOUND` | Tool name doesn't map to any registered command |
| `MCP_ARG_VALIDATION_FAILED` | Spec or custom validation failed |
| `MCP_EXECUTION_FAILED` | `command.execute` returned `Err` |
| `MCP_INTERNAL_ERROR` | Unexpected panic in the tool handler task |
| `MCP_BIND_FAILED` | TCP bind on `host:port` failed |

## Concurrency

Each tool call is handled in a separate `tokio::spawn` task. The command registry is read-only after server start — no locking is needed for registry access. Commands whose `execute` closures maintain shared mutable state must manage their own synchronization.

## Newton-specific notes

Newton defaults MCP port to `8090` (to avoid collision with its Axum default port). When `--mcp-serve` is active, Newton's Axum server does NOT start in the same process. Use separate processes or ports for simultaneous Axum + MCP serving.

## Example

See `examples/with_mcp/src/main.rs` for a minimal runnable example.
