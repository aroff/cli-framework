# Changelog

## Unreleased

### Added

- Built-in `completion <shell>` command (bash/zsh/fish/powershell) auto-registered by `AppBuilder::build()`. Apps that already define `completion` can opt out via `AppBuilder::without_completion()`.
- `api-server` feature: versioned Axum API hosting under `/api/{version}/...` with fixed `/healthz` + `/readyz` endpoints and graceful shutdown coordination.
- `api-swagger` feature: runtime OpenAPI spec endpoint and embedded Swagger UI — serves each version's app-supplied document at `GET /api/{version}/openapi.json` (with `servers:` patch) and renders a version-switchable Swagger UI at `GET /api/docs` with no CDN dependency.
- `ApiServerBuilder::root_fallback(axum::Router)`: attach a catch-all router to handle requests not matched by any framework or application route. Intended for serving a SPA or static assets at the root on the same listener as the versioned API. Receives the configured `CorsLayer` (if any); auth is intentionally not applied by default. Framework routes always take priority over the fallback.

### Breaking

- Removed `cli_framework::auth` and `cli_framework::data_source::DataSource` (and the prelude
  re-export). These modules were not integrated into command dispatch; consumers should implement
  auth and data-refresh concerns in their application layer.

## [0.4.0] — 2026-05-04

### Breaking

- `clap-dispatch` is now included in the `default` feature set. Consumers using
  `default-features = false` who relied on the legacy hand-rolled argv loop must add
  `features = ["clap-dispatch"]` to retain access to the Clap path, or adopt `CommandSpec` on
  their commands.

### Deprecated

- The `clap-dispatch` feature flag is now a no-op (Clap dispatch is always active). The flag is
  retained for one release cycle to avoid breaking consumers who list it explicitly. It will be
  removed in v0.5.0.

### Removed

- The hand-rolled `run_with_args` implementation (formerly behind
  `#[cfg(not(feature = "clap-dispatch"))]` in `src/app/builder.rs`). Only the Clap-backed path
  remains.

### Migration

Consumers using `default-features = false` who relied on the legacy argv loop must either:

1. Add `features = ["clap-dispatch"]` to their `cli-framework` dependency, **or**
2. Add `CommandSpec` to their commands to get full Clap integration.

Unknown flags now produce a structured `Diagnostic` with code `E_UNKNOWN_FLAG` on stderr instead
of being silently ignored.
