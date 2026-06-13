# Changelog

## [0.5.1] — 2026-06-13

### Added

- MCP serve path now threads a populated `ResourceRegistry` end-to-end, so registered `ui://…`
  resources are actually served (CF-6). Previously `ResourceRegistry` and
  `CliFrameworkHandler::with_resource_registry` existed but no public serve entry point ever called
  `with_resource_registry`, so a populated registry had no route to the served handler.
  - New consumer-facing slot: `AppBuilder::with_mcp_resource_registry(Arc<ResourceRegistry>)`. The
    auto-registered `mcp serve` command now serves those resources over **both** stdio and HTTP
    transports (`resources/list` + `resources/read`).
  - New HTTP-side seam for apps that mount MCP into their own Axum router:
    `mcp::build_mcp_axum_router_with_resources(...)` (the existing `build_mcp_axum_router` delegates
    to it with an empty registry).
  - New lower-level serve variants that accept an `Arc<ResourceRegistry>`:
    `serve_mcp_stdio_opts_with_resources`, `serve_mcp_with_gate_opts_with_resources`,
    `transport_stdio::start_stdio_with_resources`,
    `transport_http::start_streamable_http_with_resources`,
    `transport_http::mcp_axum_router_with_resources`.
  - All changes are additive and backward compatible: existing serve signatures are unchanged and
    default to an empty registry (a tools-only server, exactly as before).

## [0.5.0] — 2026-06-12

### Added

- MCP generic per-tool `_meta` passthrough: `Command::with_meta(serde_json::Value)` attaches an
  opaque value emitted verbatim as the tool's top-level `_meta` on `tools/list`. cli-framework does
  not inspect it — the consumer owns the entire shape (e.g. UI metadata, but the framework stays
  concept-free). `Command::with_visibility(Vec<String>)` continues to tag app-only tools (the one
  field cli-framework acts on; rides in `_meta.visibility`).
- MCP generic resource serving stays in-scope but concept-free: `UiResource::with_meta(
  serde_json::Value)` attaches an opaque per-resource `_meta` emitted verbatim at
  `contents[]._meta` in `resources/read`. The `ResourceRegistry` and `CliFrameworkHandler`
  resource seams are unchanged.
- Built-in `completion <shell>` command (bash/zsh/fish/powershell) auto-registered by `AppBuilder::build()`. Apps that already define `completion` can opt out via `AppBuilder::without_completion()`.
- `api-server` feature: versioned Axum API hosting under `/api/{version}/...` with fixed `/healthz` + `/readyz` endpoints and graceful shutdown coordination.
- `api-swagger` feature: runtime OpenAPI spec endpoint and embedded Swagger UI — serves each version's app-supplied document at `GET /api/{version}/openapi.json` (with `servers:` patch) and renders a version-switchable Swagger UI at `GET /api/docs` with no CDN dependency.
- `ApiServerBuilder::root_fallback(axum::Router)`: attach a catch-all router to handle requests not matched by any framework or application route. Intended for serving a SPA or static assets at the root on the same listener as the versioned API. Receives the configured `CorsLayer` (if any); auth is intentionally not applied by default. Framework routes always take priority over the fallback.
- `ApiServerBuilder::health_version(impl Into<String>)`: override the version string reported by `GET /healthz`. By default `/healthz` reports the framework's own crate version (`env!("CARGO_PKG_VERSION")`), fixed at cli-framework's compile time; consumers can call this to make `/healthz` report THEIR version instead. Back-compatible: when unset, `/healthz` reports the framework version exactly as before.

### Fixed

- `AsyncRetryExecutor` now honors `RetryPolicy::retry_on_timeout`. Previously the flag was ignored
  and a per-attempt timeout was always retried; with `retry_on_timeout(false)` a timed-out attempt
  now fails immediately without further retries. Non-timeout operation errors continue to retry
  regardless of the flag. Added a `unit_retry` test suite covering policy backoff math, the
  sync/async executors, the async error classifier, and timeout handling.

### Breaking

- Removed the typed MCP-Apps UI vocabulary from the core command model. cli-framework is a generic
  MCP transport and must not know UI concepts. Removed `command::UiToolMeta`, `command::UiCsp`, the
  `Command::ui` field, `Command::with_ui`, and their prelude re-exports. Replaced by the opaque
  `Command::meta: Option<serde_json::Value>` + `Command::with_meta` passthrough. Likewise
  `UiResource::csp: Option<UiCsp>` / `UiResource::with_csp` are replaced by
  `UiResource::meta: Option<serde_json::Value>` / `UiResource::with_meta`. Consumers that previously
  built `with_ui(UiToolMeta { resource_uri, csp, prefer_app })` now pass the entire `_meta` value
  themselves, e.g. `with_meta(json!({"ui":{"resourceUri":"…","csp":{…},"preferApp":true}}))`.
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
