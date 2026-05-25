# API Serving and Versioning

## Overview

The `api-server` feature provides a framework-owned Axum host for versioned HTTP APIs. The `api-swagger` feature extends it with runtime OpenAPI spec endpoints and an embedded Swagger UI.

## URL shape

| Path | Description |
|------|-------------|
| `/api/{version}/...` | Versioned API routes (app-supplied) |
| `/api/docs` | Swagger UI (requires `api-swagger`) |
| `/api/{version}/openapi.json` | Runtime OpenAPI spec (requires `api-swagger` and `openapi: Some(...)`) |
| `/healthz` | Health check (always present) |
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
