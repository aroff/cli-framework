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

### Serving `ui://` resources (MCP-Apps)

To serve registered `ui://…` resources alongside tools, build a `ResourceRegistry`, register
providers, wrap it in `Arc`, and hand it to the serve path. The auto-registered `mcp serve` command
serves them over **both** stdio and HTTP:

```rust
use cli_framework::mcp::resources::{ResourceRegistry, UiResource};
use std::sync::Arc;

let mut resources = ResourceRegistry::new();
resources.register_static(
    "ui://my-app/index.html",
    "App shell",
    UiResource::html("<!doctype html><title>App</title>"),
);

let app = AppBuilder::new()
    .with_version("my-app", "1.0.0")
    .with_mcp_resource_registry(Arc::new(resources)) // CF-6: served end-to-end
    .build(MyCtx)?;
```

For a custom Axum mount (e.g. via `ApiServer::mcp_router`), use the resource-aware router builder:

```rust
let router = cli_framework::mcp::build_mcp_axum_router_with_resources(
    &registry, "my-app", "/mcp", risk_policy, McpToolExportPolicy::AllCommands, Arc::new(resources),
);
```

When no registry is supplied, MCP serves a tools-only server (backward compatible) and advertises the
`resources` capability only once at least one resource is registered.

### Empty tool set

When `ExposeMcpOnly` is active and no commands have `expose_mcp: true`, the server starts normally with zero tools and emits a `tracing::warn!`. This is a valid operational state — the warning helps diagnose accidental misconfiguration.

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

## Per-caller tool sets (`with_mcp_dynamic_tools`) — ADR 0081

The static tool set is fixed at build time. To let different callers discover different tools,
install one hook that maps the request's identity to extra commands:

```rust
AppBuilder::new()
    .with_version("myapp", "0.1.0")
    // Establishes WHO the caller is (opaque to the framework).
    .with_mcp_request_authenticator(Arc::new(|headers| { /* -> Option<Arc<dyn Any + Send + Sync>> */ }))
    // Supplies WHAT that caller may discover, per request.
    .with_mcp_dynamic_tools(Arc::new(|identity| {
        Box::pin(async move { /* -> Vec<(String, Command)> */ })
    }))
```

Signature:

```rust
pub type McpDynamicToolsFuture =
    Pin<Box<dyn Future<Output = Vec<(String, Command)>> + Send + 'static>>;
pub type McpDynamicToolProvider =
    Arc<dyn Fn(Option<Arc<dyn Any + Send + Sync>>) -> McpDynamicToolsFuture + Send + Sync>;
```

Rules the framework guarantees:

- **One hook, both paths.** The same provider feeds `tools/list`
  (`McpToolRegistry::list_tools_for_identity`) and `tools/call`, so the advertised set and the
  callable set cannot drift.
- **Static wins.** A name registered at construction always resolves to its static command; on a
  static hit the provider is not consulted at all. A colliding name appears in `tools/list` once,
  as the static entry, and the dropped pair is reported at `tracing::warn!` naming the tool.
- **Deterministic order.** The static block keeps the order `list_tools()` produced (it is *not*
  sorted as a side effect of this feature); the per-caller block is appended after it in the order
  the provider returned. Names repeated within one provider result are skipped — first pair wins,
  also with a `tracing::warn!`. Nothing is rejected: the rest of the result is listed as normal.
- **Never cached.** Run once per `tools/list`, and once per `tools/call` that misses the static
  set. The miss rate is chosen by the caller (an unauthenticated one too), so unknown-name floods
  are a rate-limiting question at the edge. Revocation is prompt for `tools/call` and lagging for
  `tools/list`: the server advertises `tools` without `listChanged` and never sends
  `notifications/tools/list_changed`, so a client that listed once keeps showing a withdrawn tool
  until it re-lists.
- **`None` identity is normal.** Under stdio, with no authenticator installed, or when the
  authenticator rejects the credentials, the hook is called with `None`.
- **Names must already follow the convention.** The provider returns the full tool name
  (`{app_name}_{path_underscored}`); the framework does not prefix it.
- **Descriptors are generated identically.** Per-caller commands go through the same
  `command_to_tool_descriptor_full`, so `description`, `inputSchema`, `_meta` and `visibility`
  behave exactly as for a static command.
- **No hook installed → unchanged.** Both paths behave byte-identically to before on every
  transport, and `tools/list` does not invoke an installed `McpRequestAuthenticator` at all (it
  could not use the result, and running it would be a new side effect in consumer code).

- **`McpToolExportPolicy` does not filter the hook.** `expose_mcp` / `ExposeMcpOnly` are applied
  when the *static* set is built. A command the provider returns is exported as a tool even if its
  `expose_mcp` is `false`. The provider is the only filter for its own commands.
- **The risk policy and the MCP tool gate do apply.** A per-caller tool dispatches through the same
  `CommandAsToolBridge` as a static one, so `with_mcp_tool_gate` and the command risk tiers cover
  it unchanged. The gate cannot do per-caller authorization, though:
  `ExecutionGate::before_execute` receives the command, its arguments and its risk tier, not the
  identity. And `CommandRiskPolicy::classify` keys on `Command.id` plus category, so a
  provider-built command — whose id is not in the consumer's `tiers` map — takes `default_tier`
  (`Safe`) unless the provider sets a category on the command it returns.

**Discovery, not authorization.** Omitting a tool from a caller's list does not prevent them
calling it — clients may send `tools/call` with any name, tool names are guessable, and every
static command stays callable by everyone. Enforce authorization inside the command's `execute`,
the only place the caller identity is available (`ctx.request_identity::<T>()`).

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
