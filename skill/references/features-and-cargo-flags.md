# Features and Cargo flags

All 9 optional features for `cli-framework`. Default feature set includes `clap-dispatch`.

## Feature table

| Feature | Default | Description |
|---------|---------|-------------|
| `mcp-server` | off | Expose registered commands as MCP tools via Streamable HTTP; pulls in `rmcp` and `axum` |
| `clap-dispatch` | **on** | No-op since v0.4.0 (Clap dispatch is now always active); retained for compatibility, will be removed in v0.5.0 |
| `testkit` | off | Enable `CliTestHarness` for in-process CLI testing (dev/test use only) |
| `strict-types` | off | Reject registration of commands without a `CommandSpec` |
| `strict-args` | off | Reject unknown flags on legacy (no-spec) commands |
| `table-advanced` | off | Enable `comfy-table` based advanced table rendering |
| `progress` | off | Enable `indicatif` progress bars |
| `legacy-arg-coercion` | off | Coerce bare `--flag` to `Bool(true)` on legacy (no-spec) path |
| `observability` | off | Stub gate for future OpenTelemetry integration (no-op currently; see `Cargo.toml` comment) |

## Enabling combinations

```toml
[dependencies]
cli-framework = { git = "https://github.com/aroff/cli-framework", features = [
    "mcp-server",
    "testkit",
] }
```

For strict mode (enforce specs on all commands):

```toml
cli-framework = { git = "...", features = ["strict-types", "strict-args"] }
```

## Dev / test only

`testkit` should only appear in `[dev-dependencies]` or behind a `#[cfg(test)]` gate to avoid shipping test scaffolding in production binaries:

```toml
[dev-dependencies]
cli-framework = { git = "...", features = ["testkit"] }
```
