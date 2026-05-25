# API Serving and Versioning

## Overview

The `api-server` feature provides a framework-owned Axum host for versioned HTTP APIs. The `api-swagger` feature extends it with runtime OpenAPI spec endpoints and an embedded Swagger UI.

## URL shape

| Path | Description |
|------|-------------|
| `/api/{version}/...` | Versioned API routes (app-supplied) |
| `/api/docs` | Swagger UI (requires `api-swagger`) |
| `/api/{version}/openapi.json` | Runtime OpenAPI spec (requires `api-swagger` and `openapi: Some(...)`) |
| `/healthz` | Health check (always present); reports a `version` (see `health_version` below) |
| `/readyz` | Readiness check (always present) |

## Registering versions

```rust
let server = ApiServerBuilder::new()
    .version(ApiVersion {
        name: ApiVersionName::parse("v1")?,
        router: my_v1_router,
        stability: Stability::Stable,
        deprecation: None,
        #[cfg(feature = "api-swagger")]
        openapi: Some(serde_json::json!({ "openapi": "3.0.3", ... })),
    })
    .default_version(DefaultVersion::Pinned(ApiVersionName::parse("v1")?))
    .build();
```

Version names must match `^v\d+(?:beta\d+|alpha\d+)?$` (e.g., `v1`, `v2`, `v2beta1`).

## Attaching OpenAPI documents (`api-swagger`)

Enable with `features = ["api-server", "api-swagger"]`.

Set `openapi: Some(your_doc)` on each `ApiVersion` to expose that version's spec. Versions with `openapi: None` get no spec endpoint and are omitted from the Swagger UI switcher.

**`servers:` patch**: The framework replaces the `servers:` array in the served document with `[{"url": "/api/{version}"}]`. All other fields are passed through verbatim.

**Swagger UI**: `GET /api/docs` serves a fully embedded Swagger UI (no CDN dependency) with a version switcher listing every version that supplied a document. By default it opens on the `DefaultVersion::Pinned` version; if no pinned default or the pinned version has no doc, it opens on the first registered version alphabetically.

**Auth gating**: The swagger routes follow the same auth policy as the rest of the API. Auth configured via `ApiServerBuilder::auth(...)` is applied to `/api/docs/**` and `/api/{version}/openapi.json` automatically.

**Build without the flag**: When `api-swagger` is not enabled, `GET /api/docs` and `GET /api/{version}/openapi.json` both return 404 and no Swagger UI assets are embedded in the binary.

## Version lifecycle

| Field | Purpose |
|-------|---------|
| `stability` | `Stable`, `Beta`, or `Alpha` — informational |
| `deprecation` | Optional `DeprecationInfo { sunset, docs_url }` — adds `Deprecation`, `Sunset`, and `Link` response headers |

## Root fallback (SPA / static assets)

`ApiServerBuilder::root_fallback(router: axum::Router) -> Self`

Attach a catch-all router that handles any request not matched by a framework-owned route, a registered version route, a `mount()` route, MCP routes, or Swagger routes. Designed for serving a single-page app or static assets on the same listener as the versioned API.

**Route priority guarantee:** All framework routes (health, versioned API, unversioned `/api`, mounts, MCP, Swagger) are registered before the fallback. Axum's `fallback_service` only activates when no other route matches, so framework routes always win.

**Auth policy:** The host does NOT apply the `auth()` layer to the fallback. Static UI assets are typically public. If you need to gate your SPA, add auth inside the router you pass to `root_fallback()`.

**CORS:** When `cors()` is configured on the builder, the same `CorsLayer` is applied to the fallback router.

**Take-last semantics:** Calling `root_fallback()` more than once silently overwrites the previous value (consistent with `cors()` and `auth()`).

**Usage example (SPA with `tower-http`):**

```rust
// Add to Cargo.toml: tower-http = { version = "0.6", features = ["fs"] }
use tower_http::services::{ServeDir, ServeFile};
use cli_framework::axum::Router;

// Serve files from ./dist; fall back to index.html for SPA deep links.
let spa = Router::new().fallback_service(
    ServeDir::new("dist").fallback(ServeFile::new("dist/index.html")),
);

let server = ApiServerBuilder::new()
    .version(/* ... */)
    .root_fallback(spa)
    .build();

server.serve("0.0.0.0:8080").await?;
```

The framework does not add `tower-http`'s `fs` feature as a dependency; consumers add it to their own `Cargo.toml`. Any `axum::Router` is accepted — `ServeDir` is one option, not a requirement.

## Health version override

`ApiServerBuilder::health_version(v: impl Into<String>) -> Self`

`GET /healthz` returns `{"status":"ok","version": "<version>"}`. By default `<version>` is the framework's own crate version (`env!("CARGO_PKG_VERSION")`), which is fixed at cli-framework's compile time — so without an override it reports cli-framework's version, not the consumer's.

Call `health_version(...)` to make `/healthz` report your application's version instead:

```rust
let server = ApiServerBuilder::new()
    .version(/* ... */)
    .health_version(env!("CARGO_PKG_VERSION")) // your crate's version
    .build();
```

**Back-compat:** when `health_version` is not called, `/healthz` reports the framework's `CARGO_PKG_VERSION` exactly as before. **Take-last semantics:** calling it more than once overwrites the previous value.

## Error codes

| Code | Trigger |
|------|---------|
| E014 | Invalid version name |
| E015 | Duplicate version name |
| E016 | Pinned default version not registered |
| E017 | No versions registered |
| E018 | Mount path collides with reserved path (including `/api/docs`, `/api/{version}/openapi.json`) |
| E019 | Version name is a reserved segment (`docs`, `openapi.json`) |
| E020 | Unversioned request with `DefaultVersion::None` |
| E021 | Readiness check fails or server is shutting down |
| E022 | Failed to serialize app-supplied OpenAPI document at build time (`api-swagger`) |
