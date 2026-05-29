# MCP Streamable HTTP reference

Full reference for the MCP server mode in `cli-framework`. See also [`skill/examples/with_mcp`](../examples/with_mcp/).

## Enabling

```toml
[dependencies]
cli-framework = { git = "https://github.com/aroff/cli-framework", features = ["mcp-server"] }
```

## Starting the MCP server

Use the `mcp serve` subcommand:

```bash
my-app mcp serve
my-app mcp serve --host 0.0.0.0 --port 9000 --path /mcp
```

| Flag | Default | Description |
|------|---------|-------------|
| `--host` | `127.0.0.1` | Bind host |
| `--port` | `8080` | Bind port |
| `--path` | `/mcp` | HTTP path for the MCP endpoint |

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

Newton defaults MCP port to `8090` to avoid collision with its Axum default port. Use separate processes or ports for simultaneous Axum + MCP serving.

## Selective MCP exposure (`expose_mcp` and `McpToolExportPolicy`)

By default all registered commands are exported as MCP tools (`AllCommands` policy). To expose only specific commands, use `McpToolExportPolicy::ExposeMcpOnly` and flag each command you want to expose.

### `expose_mcp` field on `Command`

Every `Command` has a `bool` field `expose_mcp` (default `false`). Under `ExposeMcpOnly`, only commands where `expose_mcp == true` appear in MCP tool listings. Under `AllCommands` (the default), the field is ignored.

```rust
Command {
    id: "deploy",
    summary: "Deploy app",
    expose_mcp: true,  // visible to MCP clients when ExposeMcpOnly is active
    // ...
}

Command {
    id: "admin-reset",
    summary: "Reset admin state",
    expose_mcp: false, // excluded from MCP (default)
    // ...
}
```

Use the builder method for ergonomic opt-in:

```rust
my_command.with_expose_mcp(true)
```

### `McpToolExportPolicy` enum

```rust
pub enum McpToolExportPolicy {
    AllCommands,   // expose everything (backward-compatible default)
    ExposeMcpOnly, // expose only commands with expose_mcp: true
}
```

### `with_mcp_export_policy` on `AppBuilder`

```rust
AppBuilder::new()
    .with_version("my-app", "1.0.0")
    .with_mcp_export_policy(McpToolExportPolicy::ExposeMcpOnly)
    .register_command(Command { id: "deploy", expose_mcp: true, /* ... */ })?
    .build(MyCtx)?;
```

### `build_mcp_axum_router` and `serve_mcp`

Both functions now require an `export_policy` argument:

```rust
build_mcp_axum_router(&registry, "my-app", "/mcp", risk_policy, McpToolExportPolicy::AllCommands);
serve_mcp(registry, "my-app", args, risk_policy, McpToolExportPolicy::ExposeMcpOnly).await;
```

Pass `McpToolExportPolicy::AllCommands` to preserve existing behavior.

### Empty tool set

When `ExposeMcpOnly` is active and no commands have `expose_mcp: true`, the server starts normally with zero tools and emits a `log::warn!`. This is a valid operational state — the warning helps diagnose accidental misconfiguration.

### Framework built-in commands

The built-in `spec` and `doctor` commands are constructed with `expose_mcp: false`. They are excluded from MCP tool listings under `ExposeMcpOnly` without any consumer action.

### Migration note for struct literals

Adding `expose_mcp` to `Command` is a breaking change for struct literal construction. Add `expose_mcp: false` to every existing `Command { ... }` literal:

```rust
// Before
Command { id: "foo", summary: "...", /* other fields */ execute: ... }

// After
Command { id: "foo", summary: "...", expose_mcp: false, /* other fields */ execute: ... }
```

## Minimal snippet

```rust
// Enable with features = ["mcp-server"]
// Then launch: ./my-app mcp serve --port 9000
// All registered commands become MCP tools automatically.
use cli_framework::prelude::*;
use std::sync::Arc;

// In main: builder.with_version("my-app", env!("CARGO_PKG_VERSION"))
// This sets the MCP tool name prefix to "my-app"
```
