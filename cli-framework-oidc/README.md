# cli-framework-oidc

OIDC/OAuth2 integration for [cli-framework](https://github.com/aroff/cli-framework).

Two independent features — enable only what your application needs:

| Feature | What it provides |
|---------|-----------------|
| `client` | `OidcClient` — three OAuth2 flows + on-disk token cache; implements `TokenProvider` |
| `server` | `oidc_validation_layer` — JWT validation middleware for Axum; `OidcClaims` extractor |

## Client (`client` feature)

`OidcClient` implements `cli_framework::auth::TokenProvider`. Wire it into your app with
`AppBuilder::with_token_provider` and the four `auth` commands (`auth login`, `auth logout`,
`auth status`, `auth token`) are registered automatically.

```toml
[dependencies]
cli-framework = { version = "0.5", features = ["auth"] }
cli-framework-oidc = { version = "0.1", features = ["client"] }
```

```rust
use cli_framework::prelude::*;
use cli_framework_oidc::client::{OidcClient, OidcFlow};
use std::sync::Arc;

struct AppCtx;
impl AppContext for AppCtx {}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let client = OidcClient::builder()
        .issuer_url("https://auth.example.com")
        .client_id("my-cli")
        .flow(OidcFlow::DeviceCode)
        .build()?;

    let mut app = AppBuilder::new()
        .with_version("my-app", "1.0.0")
        .with_token_provider(Arc::new(client))
        .build(AppCtx)?;

    app.run().await
}
```

### Supported flows

| Flow | `OidcFlow` variant | When to use |
|------|-------------------|-------------|
| Device Code | `DeviceCode` | Headless / CI environments; user completes auth in a browser on another device |
| Auth Code + PKCE | `AuthCodePkce { redirect }` | Desktop apps; opens a local loopback listener, launches browser |
| Client Credentials | `ClientCredentials { client_secret, token_auth }` | Machine-to-machine; non-interactive, no user |

### Token cache

Tokens are stored in a JSON file alongside a sidecar lock file. Default location: the
`cache_dir` you provide to the builder. The cache key is a SHA-256 hash of
`{issuer}\n{client_id}\n{flow_kind}\n{sorted_scopes}` so different flows for the same
client never collide.

`access_token: null` in the cache means the token has been invalidated but the refresh
token may still be usable.

## Server (`server` feature)

`oidc_validation_layer` returns a Tower layer that validates `Authorization: Bearer` JWTs
against the issuer's JWKS endpoint. Validated claims are injected into request extensions
and accessed via the `OidcClaims` axum extractor.

```toml
[dependencies]
cli-framework = { version = "0.5", features = ["api-server"] }
cli-framework-oidc = { version = "0.1", features = ["server"] }
```

```rust
use cli_framework_oidc::server::{OidcValidationConfig, AudiencePolicy, oidc_validation_layer};
use cli_framework_oidc::OidcConfigError;

let layer = oidc_validation_layer(
    OidcValidationConfig::new(
        "https://auth.example.com",
        AudiencePolicy::Require("my-api".to_string()),
    )
    .await?,
)?;
```

Use `OidcClaims` in any handler that sits behind the layer:

```rust
use cli_framework_oidc::server::OidcClaims;
use cli_framework::axum::{Json, extract::Extension};

async fn protected(claims: OidcClaims) -> Json<serde_json::Value> {
    Json(serde_json::json!({ "sub": claims.sub }))
}
```

`OidcClaims` returns HTTP 401 with a structured `error_description` for all token
rejections (`expired`, `invalid_signature`, `unknown_key`, etc.) and HTTP 500 if the
layer is not installed (wiring bug).

### JWKS cache

The layer caches JWKS keys in memory with a configurable TTL (default 300 s). On cache
miss or key rotation it performs a single-flight refresh. If the JWKS endpoint is
unreachable it serves stale keys rather than failing all requests; it returns 503 only
when no keys have ever been fetched. Forced refetches (unknown key ID) are rate-limited
to once per 60 s by default.

## License

Apache-2.0 — same as `cli-framework`.
