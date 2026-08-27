# Features and Cargo flags

All optional features for `cli-framework`. Default feature set includes `chat`.

## Feature table

| Feature | Default | Description |
|---------|---------|-------------|
| `chat` | **on** | Multi-turn agentic command resolution via `aikit-agent`; provides the `chat` command |
| `mcp-server` | off | Expose registered commands as MCP tools via Streamable HTTP; pulls in `rmcp` and `axum` |
| `api-server` | off | Versioned Axum API hosting under `/api/{version}/...` with health/readiness endpoints and graceful shutdown |
| `api-swagger` | off | Runtime OpenAPI spec endpoint at `/api/{version}/openapi.json` + embedded Swagger UI at `/api/docs`; requires `api-server` |
| `auth` | off | Generic `TokenProvider` trait + `AuthenticatedHttpClient` + four auto-registered `auth` subcommands; pair with `cli-framework-oidc` for OIDC flows |
| `doctor` | off | Structured `DoctorCheck` trait, concurrent runner, and `doctor` CLI command with terminal/JSON output |
| `project-config` | off | Project root discovery and TOML config loading |
| `testkit` | off | Enable `CliTestHarness` for in-process CLI testing (dev/test use only) |
| `table-advanced` | off | Enable `comfy-table` based advanced table rendering |
| `progress` | off | Enable `indicatif` progress bars |
| `observability` | off | `tracing-subscriber` logging foundation; implied by `telemetry` |
| `telemetry` | off | Built-in OpenTelemetry (ADR 0068). Auto `cli.command` spans at the CLI + MCP dispatch seams plus `cli.command.invocations` / `cli.command.duration_ms` metrics tagged `{command, surface, status}`, exported over OTLP HTTP; `TelemetryConfig`, `AppBuilder::with_telemetry` / `ApiServerBuilder::with_telemetry`, `ctx.telemetry()` handle. Implies `observability`. **`with_telemetry()` installs a process-global `tracing` subscriber** — if the app already installed its own, it warns on stderr and exports nothing; compose `telemetry::init::otel_layer(&guard)` instead. No context propagation and no OTLP auth headers yet |

## `cli-framework-oidc` companion crate

A separate crate that provides OIDC/OAuth2 flows. Two independent features — enable only what you need:

| Feature | What it provides |
|---------|-----------------|
| `client` | `OidcClient` — three OAuth2 flows (Device Code, Auth Code PKCE, Client Credentials) + on-disk token cache; implements `TokenProvider` |
| `server` | `oidc_validation_layer` — Tower/Axum JWT validation middleware + `OidcClaims` extractor; JWKS cache with TTL, single-flight, serve-stale-on-error |

```toml
# CLI that lets users log in
[dependencies]
cli-framework = { git = "...", features = ["auth"] }
cli-framework-oidc = { path = "../cli-framework-oidc", features = ["client"] }

# API server validating incoming JWTs (no login, no TokenProvider)
[dependencies]
cli-framework = { git = "...", features = ["api-server"] }
cli-framework-oidc = { path = "../cli-framework-oidc", features = ["server"] }
```

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
