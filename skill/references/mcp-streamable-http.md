# MCP Streamable HTTP reference

Full reference for the MCP server mode in `cli-framework`. See also [`skill/examples/with_mcp`](../examples/with_mcp/).

## Enabling

```toml
[dependencies]
cli-framework = { git = "https://github.com/aroff/cli-framework", features = ["mcp-server"] }
```

## Runtime flags

| Flag | Default | Description |
|------|---------|-------------|
| `--mcp-serve` | (off) | Start MCP Streamable HTTP server instead of normal CLI dispatch |
| `--mcp-host` | `127.0.0.1` | Bind host |
| `--mcp-port` | `8090` | Bind port |
| `--mcp-path` | `/mcp` | HTTP path for the MCP endpoint |

Example:

```bash
my-app --mcp-serve --mcp-host 0.0.0.0 --mcp-port 9000 --mcp-path /mcp
```

## Tool naming convention

Tool names follow `<app_name>.<command_id>`:
- `app_name` comes from `AppBuilder::with_version(name, _)`
- Hierarchical commands: `cluster/get` → `myapp.cluster.get` (slashes become dots)
- If `app_name` is `"unknown"`, a startup warning is emitted

## Validation pipeline

MCP tool calls are routed through the same pipeline as CLI calls:

1. `SpecValidator` — required args, type conformance, conflicts, `requires` constraints
2. Custom `validator` — if registered on the `Command`
3. Risk policy — `CommandRiskPolicy` checks (Safe / Sensitive / Destructive)

There is no way to bypass validation from an MCP client.

## Error codes

| Code | When |
|------|------|
| `MCP_CMD_NOT_FOUND` | Tool name doesn't map to any registered command |
| `MCP_ARG_VALIDATION_FAILED` | Spec or custom validation failed |
| `MCP_EXECUTION_FAILED` | `command.execute` returned `Err` |
| `MCP_INTERNAL_ERROR` | Unexpected panic in the tool handler task |
| `MCP_BIND_FAILED` | TCP bind on `host:port` failed (port in use, permission denied) |

## Concurrency model

Each tool call is handled in a separate `tokio::spawn` task. The command registry is read-only after server start — no locking is needed for registry access. Commands whose `execute` closures maintain shared mutable state must manage their own synchronization (e.g. `Arc<Mutex<_>>`).

## Newton-specific notes

Newton defaults MCP port to `8090` to avoid collision with its Axum default port. When `--mcp-serve` is active, Newton's Axum server does NOT start in the same process. Use separate processes or ports for simultaneous Axum + MCP serving.

## Minimal snippet

```rust
// Enable with features = ["mcp-server"]
// Then launch: ./my-app --mcp-serve --mcp-port 9000
// All registered commands become MCP tools automatically.
use cli_framework::prelude::*;
use std::sync::Arc;

// In main: builder.with_version("my-app", env!("CARGO_PKG_VERSION"))
// This sets the MCP tool name prefix to "my-app"
```
