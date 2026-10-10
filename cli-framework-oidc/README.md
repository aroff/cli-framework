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

### Explicit SSO logout

`TokenProvider::logout()` clears local credentials only. Native applications may
separately call `client.end_session_url(post_logout_redirect_uri, state).await`
to prepare the issuer's advertised RP-initiated logout URL. The host opens or
displays that URL; preparing it does not terminate SSO or clear the token cache.
Clear local credentials independently, including when discovery fails.

Return URIs must be registered with the provider. If supplying state, the host
must validate it on return. This helper sends no bearer or refresh token and does
not retain an ID-token hint, so the provider may request browser confirmation.
Report logout as requested, not verified; existing access tokens may remain valid.
Discovery requires credential-free HTTPS (loopback HTTP is allowed for tests),
does not follow redirects, and validates the returned issuer.

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

let layer = oidc_validation_layer(OidcValidationConfig::new(
    "https://auth.example.com",
    AudiencePolicy::Require("my-api".to_string()),
))?;
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
rejections (`expired`, `invalid_signature`, `unknown_key`, `unknown_issuer`, etc.) and HTTP 500 if the
layer is not installed (wiring bug).

### Several trusted issuers

`OidcValidator::new_multi` (or `oidc_validation_layer_multi`) trusts a list of
issuers, one `OidcValidationConfig` each, with its own audience policy, JWKS
source, algorithms and claim paths (ADR 0082):

```rust
use cli_framework_oidc::server::{AudiencePolicy, OidcValidationConfig, OidcValidator};

let validator = OidcValidator::new_multi([
    OidcValidationConfig::new(
        "https://login.example.com/realms/users",
        AudiencePolicy::Require("my-api".into()),
    )
    .with_groups_claim_path("groups"),
    OidcValidationConfig::new(
        "https://workload-issuer.example.net",
        AudiencePolicy::Require("https://my-api.example.com".into()),
    )
    .with_roles_claim_path("org.roles"),
])?;
let layer = validator.layer(); // same caches as `validator.validate(..)`
```

A token is routed by its `iss` claim, read from the payload before any
verification, to the issuer whose normalized `issuer_url` equals it exactly.
That issuer then verifies the token in full (its algorithms, its own keys, `iss`,
`aud`, `exp`), so a key published by one issuer never validates a token claiming
another. A token whose `iss` names no configured issuer is rejected with
`error_description="unknown_issuer"` (`TokenRejection::UnknownIssuer`) without
any discovery or JWKS request. Each issuer keeps its own JWKS cache, refetch
rate limit and single-flight gate. Two configs that normalize to the same issuer
are a construction error (`OidcConfigError::DuplicateIssuer`). Handlers tell
issuers apart by `OidcClaims::iss`, the normalized configured issuer.

`OidcValidator::new` / `oidc_validation_layer` (one issuer) behave as before.
Both forms require `iss`: a token with no `iss`, or with a non-string `iss`, is
rejected (`unknown_issuer` with several issuers, `invalid_issuer` with one).
With `AudiencePolicy::Require` or `RequireAny`, a token with no `aud` is
rejected as `invalid_audience`; only `Unchecked` accepts it.

### Roles and groups claim paths

`OidcClaims::roles` is read from `roles_claim_path` (default
`realm_access.roles`, where Keycloak puts realm roles) and `OidcClaims::groups`
from `groups_claim_path` (default none, so `groups` is empty). Both are set per
issuer:

- A path is object keys separated by `.`, descending through nested JSON
  objects: `org.roles` reads `{"org": {"roles": [...]}}`. Arrays are not
  descended into.
- `\.` is a literal dot inside a key, `\\` a literal backslash:
  `https://example\.com/groups` is the single key `https://example.com/groups`.
  In Rust source, write it as a raw string: `r"https://example\.com/groups"`.
- The value is read as a list of strings: the string elements of an array, or a
  single string as a one-element list. Anything else, or a path that does not
  resolve, gives an empty list, not an error.
- An empty path, an empty key (`a..b`), an unknown escape or a trailing
  backslash is rejected when the validator is built
  (`OidcConfigError::InvalidClaimPath`).

The browser layers (`browser` feature) read roles from the default path and
leave `groups` empty.

### Inline keys and tests without an identity provider

`OidcValidationConfig::static_jwks` (or `.with_static_jwks(jwk_set)`) gives an
issuer's keys inline as a `JwkSet` (re-exported from `jsonwebtoken`). Such an
issuer never performs discovery or a JWKS fetch. It cannot be combined with
`jwks_uri`, must hold at least one key, and refuses symmetric (`oct`) keys.

With the `test-support` feature (for `[dev-dependencies]` only),
`test_support::TestIssuer` synthesizes issuers this way, so tests need no
network and no real identity provider:

```rust
use cli_framework_oidc::server::OidcValidator;
use cli_framework_oidc::test_support::TestIssuer;
use serde_json::json;

let a = TestIssuer::new("https://issuer-a.test");
let b = TestIssuer::new("https://issuer-b.test");
let validator = OidcValidator::new_multi([
    a.config().with_groups_claim_path("groups"),
    b.config(),
])?;
let claims = validator
    .validate(&a.mint(json!({"sub": "alice", "groups": ["/team"]})))
    .await?;
assert_eq!((claims.iss.as_str(), claims.groups), ("https://issuer-a.test", vec!["/team".to_string()]));
```

`mint` fills `iss`, `iat` and `exp` when the claims leave them out; set `iss`
yourself to forge a token for another issuer.

### JWKS cache

The layer caches JWKS keys in memory with a configurable TTL (default 300 s). On cache
miss or key rotation it performs a single-flight refresh. If the JWKS endpoint is
unreachable it serves stale keys rather than failing all requests; it returns 503 only
when no keys have ever been fetched. Forced refetches (unknown key ID) are rate-limited
to once per 60 s by default. With several issuers, each has its own cache and limits.

## Host session (`host-session` feature)

A web host that signs the end user in on the server and keeps their tokens out
of the browser. The host is a confidential client; the tokens are sealed into
one `__Host-session` cookie (`HttpOnly; Secure; SameSite=Lax; Path=/`) that the
browser can't read.

```rust,ignore
use cli_framework_oidc::host_session::{ClientSecret, HostSessionConfig, HostSessions, Resolution, SessionKey};

let mut cfg = HostSessionConfig::new(
    "https://auth.example.com/realms/acme",
    "acme-host",
    "https://acme.example.com/_host/callback",
    SessionKey::from_bytes(key_bytes),
    "acme-prod", // the deployment this session is bound to
);
cfg.client_secret = Some(ClientSecret::new(secret));
let sessions = HostSessions::new(cfg)?; // checks the config and the cookie size

let app = Router::new().merge(sessions.router()); // /_host/{login,callback,logout,session}

// In your own handlers:
match sessions.resolve(request.headers()).await {
    Resolution::Active(s) => { /* s.access_token(), s.claims(); send s.set_cookie() if Some */ }
    Resolution::Ended { reason, clear_cookie } => { /* 401; send clear_cookie if Some */ }
}
```

- **Sign-in** is authorization code with PKCE, `state` and `nonce`, carried in
  a short-lived sealed `__Host-sign-in` cookie. `return_to` must be a local
  path. The ID token's audience, nonce and subject are checked, and the access
  token is verified against the realm's keys.
- **Authorized party**: the access token's `azp` must be the host's
  `client_id`, at sign-in and after every refresh (`require_access_azp`, on by
  default). Its `aud` names the APIs it is for, usually not the host, so this
  is what ties it to the host; with it on, the default `Unchecked`
  `access_audience` logs no warning. A refresh returning a token issued to
  another client ends the session as `RefreshRefused`. Turn it off only for a
  provider whose access tokens carry no `azp`, and set `access_audience`.
- **Idle timeout** defaults to 30 minutes. The cookie is rewritten with a new
  activity stamp once a tenth of the window has passed, not on every request.
  The cookie has no `Max-Age` unless `session_ttl` is set.
- **Binding**: a cookie sealed for another deployment string, or with another
  key, ends as `BindingMismatch` or `Invalid`.
- **Refresh**: an access token within `refresh_skew` of expiry is refreshed. A
  refused refresh ends the session; an unreachable realm keeps the old token
  until it expires, then answers `Unavailable` without clearing the cookie.
- **Logout** is a same-origin POST. It ends the realm session with the refresh
  token, clears the cookie, and redirects to the realm's end-session endpoint.
- **Cookie size**: `HostSessions::new` refuses a configuration whose expected
  tokens would not fit `max_cookie_bytes` (4096), and sign-in refuses real
  tokens that don't. The sealed size is logged as `cookie_bytes`.

## License

Apache-2.0 — same as `cli-framework`.
