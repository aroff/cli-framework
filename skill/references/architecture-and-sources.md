# Architecture and sources

Module map for `cli-framework`. Read alongside [`CONTRIBUTING.md`](../../CONTRIBUTING.md), [`README.md`](../../README.md), [`docs/migration-typed-spec.md`](../../docs/migration-typed-spec.md), and [`docs/testing.md`](../../docs/testing.md).

## Module map

| Module(s) | Role |
|-----------|------|
| `app` | `AppBuilder`, `App::run`, dispatch entry |
| `command`, `command::chat` | `Command` struct, registry, `chat` agentic resolution |
| `parser`, `spec` | argv → `CommandArgs`; `CommandPath`, `CommandSpec`, `ArgSpec` |
| `plugin` | Plugin registry TOML / manifests, `PluginRegistryManager` |
| `ailoop` | `ailoop-core` client, confirmation flow |
| `security` | Output sanitization, command risk policy (`CommandRiskPolicy`) |
| `http_retry`, `retry` | `RetryableHttpClient`, circuit breaker, `secure_reqwest_client` |
| `cli_output`, `cli_mode`, `message` | Help rendering, tables, JSON output modes |
| `auth` | Auth state persistence |
| `data_source` | Abstract data source trait |
| `observability` | Stub feature gate (see `Cargo.toml` comment) |
| `testkit` | `CliTestHarness` (feature-gated) |
| `mcp` | MCP tool registry, Streamable HTTP server (feature-gated) |

## Flow summary

```
AppBuilder::new()
  .register_command(cmd)?      // stores Command in registry
  .build(ctx)?                 // freezes registry, wraps context
  .run().await                 // parses argv → resolves id → dispatch execute
```

For `chat`:
```
user input → aikit-agent tool-calling loop → risk gate → optional confirm → same dispatch
```

## Key source paths

```
src/app/builder.rs           AppBuilder, register_command
src/command/mod.rs           Command struct
src/spec/command_tree.rs     CommandSpec, CommandPath
src/spec/arg_spec.rs         ArgSpec, ArgValueType, Cardinality
src/security/command_risk.rs  Risk tiers, ALLOW_DESTRUCTIVE_COMMANDS
src/security/output_sanitize.rs  Output sanitization
src/http_retry/client.rs     RetryableHttpClient
src/mcp/mod.rs               MCP server handler
src/testkit/                 CliTestHarness
```
