# Authentication and OIDC

Generic `TokenProvider` trait + `AuthenticatedHttpClient` in `cli-framework` (`auth` feature), with `cli-framework-oidc` providing OIDC/OAuth2 flows and on-disk token caching.

## Cargo setup

### User-facing CLI (needs login/logout/token commands)

```toml
[dependencies]
cli-framework = { git = "https://github.com/aroff/cli-framework", features = ["auth"] }
cli-framework-oidc = { path = "../cli-framework-oidc", features = ["client"] }
dirs = "5"             # for cache_dir location
async-trait = "0.1"
```

### API server validating incoming JWTs (no login needed)

```toml
[dependencies]
cli-framework = { git = "...", features = ["api-server"] }
cli-framework-oidc = { path = "../cli-framework-oidc", features = ["server"] }
```

---

## `TokenProvider` trait

`OidcClient` implements it for you. Only implement it directly for non-OIDC providers (env vars, static tokens, tests).

| Method | Contract |
|--------|----------|
| `token() -> Result<AccessToken, AuthError>` | **Non-interactive only.** Return a valid token from cache or via a silent grant (refresh, CC). Never launch a browser or prompt. Return `NotAuthenticated` if no silent path exists. |
| `invalidate()` | Best-effort, infallible. Mark the cached access token as invalid (sets it to `null`). Refresh token is preserved. |
| `peek() -> Option<TokenStatus>` | Read-only cache check. Return `None` if unsupported. Never makes a network call. |
| `login() -> Result<(), AuthError>` | Launch an interactive flow (Device Code, PKCE). Default impl returns `NotSupported`. |
| `logout() -> Result<(), AuthError>` | Clear all cached tokens. Default impl returns `NotSupported`. |

`AccessToken::as_bearer()` returns the raw string. Its `Debug` impl prints `"***"` — safe to log.

---

## `OidcClient` — Keycloak setup

> **Runnable example.** A complete, copyable CLI lives at `cli-framework-oidc/examples/keycloak_cli.rs`. Point it at your realm via env vars and run:
> ```bash
> export KC_ISSUER_URL=https://keycloak.example.com/realms/my-realm KC_CLIENT_ID=my-cli
> cargo run -p cli-framework-oidc --example keycloak_cli --features client -- auth login
> cargo run -p cli-framework-oidc --example keycloak_cli --features client -- whoami
> ```

### Keycloak client configuration

| Scenario | Client type | Settings to enable |
|----------|------------|-------------------|
| CLI with user login | Public | Standard Flow + Device Authorization Grant |
| Machine-to-machine | Confidential | Service accounts roles |

For a public client with Device Code + PKCE:
- **Valid Redirect URIs**: `http://127.0.0.1:8765/callback`
- **Standard Flow Enabled**: on
- **Device Authorization Grant Enabled**: on

### Builder

```rust
use cli_framework_oidc::client::{OidcClient, OidcFlow};

let client = OidcClient::builder()
    .issuer_url("https://keycloak.example.com/realms/my-realm")  // realm URL, no trailing slash
    .client_id("my-cli")
    .flow(OidcFlow::DeviceCode)
    .app_name("my-app")   // default cache dir → <os-cache>/cli-framework-oidc/my-app
    .build()?;
```

`issuer_url` is what Keycloak calls the **Realm URL** — it ends with `/realms/<realm-name>`. The client fetches `<issuer>/.well-known/openid-configuration` on first use (lazy, cached for the client's lifetime).

**`cache_dir` is optional.** Set `.app_name("my-app")` and the cache defaults to `<os-cache>/cli-framework-oidc/my-app` — no `dirs` dependency or path-building in your app. Pass `.cache_dir(path)` only to override.

### Config from environment

Skip hand-wiring entirely — `from_env("PREFIX")` reads `{PREFIX}_ISSUER_URL`, `{PREFIX}_CLIENT_ID`, `{PREFIX}_CLIENT_SECRET` (optional), `{PREFIX}_FLOW` (optional), `{PREFIX}_SCOPES` (optional):

```rust
use cli_framework_oidc::client::OidcClientBuilder;

let client = OidcClientBuilder::from_env("KC")?
    .app_name("my-app")
    .build()?;
```

Flow resolution: an explicit `KC_FLOW` (`device` | `pkce` | `client-credentials` | `auto`) wins; otherwise a present `KC_CLIENT_SECRET` selects Client Credentials, and its absence selects an interactive flow via `OidcFlow::auto_interactive()`.

### Automatic flow selection

`OidcFlow::auto_interactive()` picks **Auth Code + PKCE** when a local GUI/browser is available and **Device Code** over SSH or on a headless box (checks `SSH_CONNECTION`/`SSH_TTY` and `DISPLAY`/`WAYLAND_DISPLAY`). Use it instead of hard-coding one interactive flow:

```rust
.flow(OidcFlow::auto_interactive())
```

### Available flows

```rust
// Headless / SSH / CI: user completes login on another device
OidcFlow::DeviceCode

// Desktop: opens browser on localhost, PKCE S256, loopback on port 8765
OidcFlow::AuthCodePkce {
    redirect: RedirectConfig::default(),   // Fixed(8765)
}

// Machine-to-machine (confidential client with client_secret)
use secrecy::SecretString;
use cli_framework_oidc::client::TokenAuthMethod;
OidcFlow::ClientCredentials {
    client_secret: SecretString::new("...".into()),
    token_auth: TokenAuthMethod::Post,     // or Basic
}
```

**Default scopes**: `["openid"]` for DeviceCode/PKCE, `[]` for ClientCredentials. Override with `.scopes(vec!["openid", "profile", "email"])`.

---

## Token cache

### File location

The cache file is `oidc-token.json` inside whatever path you pass to `.cache_dir(...)`.

Recommended: `dirs::cache_dir().unwrap().join("<app-name>")` → `~/.cache/<app-name>/oidc-token.json` on Linux/macOS.

A sidecar `oidc-token.lock` is created next to it on first write and used for cross-process `flock` serialization — two concurrent CLI invocations won't corrupt the file.

On unix the token file and its lock are created with mode `0600` (owner read/write only) under a `0700` parent directory, so other local users cannot read cached bearer/refresh tokens.

### File schema

```json
{
  "version": 1,
  "entries": {
    "<sha256-key>": {
      "access_token": "eyJhbGc...",   // null after invalidate()
      "refresh_token": "eyJhbGc...",  // preserved after invalidate()
      "expires_at": "2026-06-17T15:00:00Z",
      "obtained_at": "2026-06-17T14:00:00Z",
      "scopes": ["openid"]
    }
  }
}
```

The cache key is `lowercase_hex(SHA-256("{issuer}\n{client_id}\n{flow_kind}\n{sorted_scopes}"))`. Different flows/realms/scopes never collide.

`access_token: null` means the token was invalidated (e.g. after a 401 response) but the refresh token is still available for a silent re-acquire on the next `token()` call.

### Runtime caching logic

`token()` runs this decision tree on every call — no network if the cache is fresh:

```
cache hit + access_token + not near expiry (default 60s skew buffer)?
    → return cached token immediately

refresh_token in cache?
    → POST /token  grant_type=refresh_token
    → store new access/refresh, return

ClientCredentials flow?
    → POST /token  grant_type=client_credentials
    → store, return

else → NotAuthenticated  (user must run `auth login`)
```

---

## Wiring into `AppBuilder`

```rust
use cli_framework::prelude::*;
use cli_framework_oidc::client::{OidcClient, OidcFlow};
use std::sync::Arc;

struct AppCtx;
impl AppContext for AppCtx {}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cache_dir = dirs::cache_dir().unwrap().join("my-app");

    let oidc = OidcClient::builder()
        .issuer_url("https://keycloak.example.com/realms/my-realm")
        .client_id("my-cli")
        .flow(OidcFlow::DeviceCode)
        .cache_dir(cache_dir)
        .build()?;

    let mut app = AppBuilder::new()
        .with_version("my-app", "1.0.0")
        .with_token_provider(Arc::new(oidc))   // ← registers auth commands
        .build(AppCtx)?;

    app.run().await
}
```

Calling `.with_token_provider(...)` auto-registers four commands:

| Command | What it does |
|---------|-------------|
| `auth login` | Calls `TokenProvider::login()` — runs the interactive flow (Device Code prints URL+code; PKCE opens browser) |
| `auth logout` | Removes the cache entry entirely |
| `auth status` | Reads cache via `peek()` then tries a silent `token()` call; `--json` for scripting; `--no-refresh` to skip network |
| `auth token` | Prints the raw bearer to **stdout** (exit 0); exits 1 with AUTH003 if not authenticated — useful for `curl -H "Authorization: Bearer $(my-app auth token)"` |

Auth commands are never exposed as MCP tools or chat tools.

---

## `AuthenticatedHttpClient`

Wraps `RetryableHttpClient`. Injects `Authorization: Bearer <token>` on every request. On a 401 response it calls `invalidate()` (nulls the access token, preserves refresh), re-calls `token()` (which tries the refresh grant), and re-issues the request once. If the second attempt also returns 401, the error propagates.

```rust
use cli_framework::auth::AuthenticatedHttpClient;
use cli_framework::http_retry::RetryableHttpClient;

// Typically stored on AppContext
struct AppCtx {
    api: AuthenticatedHttpClient,
}

// Construction (at startup, after building the provider)
let provider: Arc<dyn TokenProvider> = Arc::new(oidc_client);
let http = AuthenticatedHttpClient::new(
    RetryableHttpClient::new(reqwest::Client::new()),
    provider,
);

// In a command handler
let resp = http.get("https://api.example.com/things").await?;
```

The client exposes `get`, `post`, `put`, `delete`, `patch`, `head`, `options`. Each takes a `&str` URL and returns `anyhow::Result<reqwest::Response>`.

### Accessing the provider from a command

If a command needs the token directly (e.g. to pass to a library that takes a raw bearer):

```rust
execute: Arc::new(|ctx, _args| Box::pin(async move {
    let provider = ctx.opt_token_provider()
        .ok_or_else(|| anyhow::anyhow!("auth not configured"))?;
    let token = provider.token().await?;
    let bearer = token.as_bearer().to_string();
    // use bearer ...
    Ok(())
})),
```

---

## Auth exit codes

| Code | When | Exit |
|------|------|------|
| `AUTH001` | `login` or `logout` called but provider returns `NotSupported` | 1 |
| `AUTH002` | Provider-level error (network failure, bad response from Keycloak) | 1 |
| `AUTH003` | `auth token` called with no valid token and no silent path | 1 |

`auth status` always exits 0 — it is a read-only query.

---

## Testing auth commands

Use a stub `TokenProvider` in tests — no Keycloak server needed:

```rust
use cli_framework::auth::{AccessToken, AuthError, TokenProvider, TokenStatus};
use cli_framework::testkit::CliTestHarness;
use std::sync::Arc;

struct FakeProvider { logged_in: bool }

#[async_trait::async_trait]
impl TokenProvider for FakeProvider {
    async fn token(&self) -> Result<AccessToken, AuthError> {
        if self.logged_in {
            Ok(AccessToken::new("test-token".into(), None))
        } else {
            Err(AuthError::NotAuthenticated)
        }
    }
    async fn invalidate(&self) {}
    async fn login(&self) -> Result<(), AuthError> { Ok(()) }
    async fn logout(&self) -> Result<(), AuthError> { Ok(()) }
    async fn peek(&self) -> Option<TokenStatus> {
        Some(TokenStatus { logged_in: self.logged_in, expires_at: None })
    }
}

#[tokio::test]
async fn auth_token_prints_bearer_to_stdout() {
    let app = AppBuilder::new()
        .with_version("myapp", "1.0")
        .with_token_provider(Arc::new(FakeProvider { logged_in: true }))
        .build(MyCtx)
        .unwrap();

    let mut h = CliTestHarness::new(app);
    let out = h.run(&["myapp", "auth", "token"]).await;

    assert_eq!(out.exit_code(), 0);
    assert_eq!(out.stdout().trim(), "test-token");
}

#[tokio::test]
async fn auth_token_not_authenticated_exits_1() {
    let app = AppBuilder::new()
        .with_version("myapp", "1.0")
        .with_token_provider(Arc::new(FakeProvider { logged_in: false }))
        .build(MyCtx)
        .unwrap();

    let mut h = CliTestHarness::new(app);
    let out = h.run(&["myapp", "auth", "token"]).await;

    assert_eq!(out.exit_code(), 1);
    out.assert_diagnostic_code("AUTH003");
}
```

---

## Local development without a live Keycloak

Three patterns — pick by need:

### Env-var bypass (recommended)

Wrap your `OidcClient` in a thin provider that checks an env var first. Zero new dependencies, no code branching, works in CI with a secret injected as `MY_APP_TOKEN`.

```rust
use cli_framework::auth::{AccessToken, AuthError, TokenProvider, TokenStatus};
use cli_framework_oidc::client::OidcClient;

pub struct KeycloakProvider {
    oidc: OidcClient,
}

#[async_trait::async_trait]
impl TokenProvider for KeycloakProvider {
    async fn token(&self) -> Result<AccessToken, AuthError> {
        // Set MY_APP_TOKEN=any-value to skip Keycloak entirely in local dev / CI.
        if let Ok(t) = std::env::var("MY_APP_TOKEN") {
            if !t.is_empty() {
                return Ok(AccessToken::new(t, None));
            }
        }
        self.oidc.token().await
    }

    async fn invalidate(&self) { self.oidc.invalidate().await }
    async fn peek(&self) -> Option<TokenStatus> { self.oidc.peek().await }
    async fn login(&self) -> Result<(), AuthError> { self.oidc.login().await }
    async fn logout(&self) -> Result<(), AuthError> { self.oidc.logout().await }
}
```

```bash
MY_APP_TOKEN=fake-dev-token cargo run -- list-things
```

### Local Keycloak via Docker

```bash
docker run -p 8080:8080 \
  -e KEYCLOAK_ADMIN=admin -e KEYCLOAK_ADMIN_PASSWORD=admin \
  quay.io/keycloak/keycloak:latest start-dev
```

Point the `OidcClient` at `http://localhost:8080/realms/master`. The `normalize_issuer` function allows `http://` for loopback addresses, so TLS is not required locally.

### Cargo feature flag

For builds where you want no Keycloak dependency at all:

```toml
[features]
dev-auth = []
```

```rust
#[cfg(feature = "dev-auth")]
let provider: Arc<dyn TokenProvider> = Arc::new(StaticTokenProvider("dev-token".into()));

#[cfg(not(feature = "dev-auth"))]
let provider: Arc<dyn TokenProvider> = Arc::new(KeycloakProvider::new(oidc_client));
```

---

## JWT validation on an API server (`server` feature)

For Axum-based API servers that need to validate tokens issued by Keycloak:

```rust
use cli_framework_oidc::server::{AudiencePolicy, OidcValidationConfig, OidcClaims, oidc_validation_layer};

let layer = oidc_validation_layer(
    OidcValidationConfig::new(
        "https://keycloak.example.com/realms/my-realm",
        AudiencePolicy::Require("my-api".into()),  // matches Keycloak client ID of the resource server
    )
    .await?,
)?;

// Apply to your Axum router
let protected = Router::new()
    .route("/things", get(list_things))
    .layer(layer);

// In handler — OidcClaims is injected by the layer
async fn list_things(claims: OidcClaims) -> impl IntoResponse {
    Json(json!({ "sub": claims.sub }))
}
```

**Keycloak audience gotcha.** A Keycloak access token's `aud` does **not** include the client by default — it's often `["account"]`. So `AudiencePolicy::Require("my-api")` will reject otherwise-valid tokens until you either (a) add an **Audience mapper** (or audience client-scope) on the client so the token's `aud` contains `my-api`, then use `Require`/`RequireAny`; or (b) start with `AudiencePolicy::Unchecked` to prove the signature/issuer/JWKS path, then tighten. `RequireAny(vec!["my-api".into(), "account".into()])` is a pragmatic middle ground.

### Tuning the validation config

`OidcValidationConfig::new(issuer, audience)` fills sane defaults; override fields before passing it to the layer:

| Field | Default | Purpose |
|-------|---------|---------|
| `audience` | (required arg) | `AudiencePolicy::Require("<aud>")` enforces an exact `aud`; `AudiencePolicy::RequireAny(vec![..])` accepts if **any** listed value is present (Keycloak's `aud` is an array, e.g. `["account", "my-api"]`); `AudiencePolicy::Unchecked` skips it and logs a WARN |
| `algorithms` | `[RS256]` | Accepted JWT signing algorithms |
| `jwks_uri` | `None` (discover) | Override to skip discovery and pin the JWKS endpoint |
| `jwks_ttl` | 300 s | How long a fetched key set is considered fresh |
| `clock_skew` | 60 s | Leeway applied to `exp`/`nbf`/`iat` |
| `min_refetch_interval` | 60 s | Floor between forced (unknown-`kid`) refetches — the rate-limit half of the amplification defense |

### JWKS fetching, key rotation, and amplification defense

JWKS keys are fetched from `<issuer>/protocol/openid-connect/certs` (resolved via discovery). The cache is keys-only (never tokens) and **serve-stale-on-error**: the layer returns **503 + `Retry-After`** only when it has *never* successfully fetched any keys.

The JWT header names the `kid` that signed the token. Keycloak rotates signing keys, so a token may legitimately carry a `kid` the cache hasn't seen yet:

- On an **unknown `kid`**, the layer forces **one** refetch (picking up the rotated key) rather than rejecting the token — so a key rotation does not cause an outage.
- Because the `kid` header is attacker-controlled and unsigned, that refetch is bounded on two axes so forged-`kid` tokens can't amplify into a fetch flood against the shared IdP (**ADR 0070**):
  - **Single-flight** — at most one JWKS fetch is in flight at any instant; concurrent refetch-needing requests share its result. Fresh-cache validations never wait on it.
  - **Rate-limit** (`min_refetch_interval`) — at most one forced refetch per interval; within the window an unknown `kid` is rejected immediately with 401 instead of fetching.

Net effect: during a real rotation, a burst of requests carrying the new `kid` all succeed via one shared refetch (no spurious 401s); a flood of distinct random `kid`s costs at most one outbound fetch per interval.

### 401 / `WWW-Authenticate` responses

A rejected token returns 401 with an RFC 6750 header naming the reason, e.g.:

```
WWW-Authenticate: Bearer error="invalid_token", error_description="expired"
```

`error_description` is drawn from a closed set: `expired`, `not_yet_valid`, `invalid_signature`, `invalid_issuer`, `invalid_audience`, `unsupported_algorithm`, `unknown_key` (the `kid` was not found even after a forced refetch), and `malformed_token` (missing `kid`/`exp`/`sub` and other structural failures). Two cases carry no `error_description`: a missing/empty `Authorization` header returns a bare `Bearer` challenge, and a token whose header can't even be decoded returns a bare `error="invalid_token"`.
