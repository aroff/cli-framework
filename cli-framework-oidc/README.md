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

## Browser middleware transport bounds

With the `browser` feature, navigation discovers `authorization_endpoint` from
the configured issuer. It preserves provider query parameters and uses URL query
encoding for OAuth parameters. Missing endpoints and conflicting reserved
parameters return 503 without issuing login state. Configure `callback_path` to
match the registered `redirect_uri`; the temporary state cookie uses that path.

The returned boxed layers wrap a `Router` service. Use `tower::Layer::layer`
and `Router::fallback_service` or `nest_service`, rather than `Router::layer`,
which applies layers to individual `Route` services.

Browser and server discovery/JWKS clients do not follow redirects. Connect and
total request deadlines are three and ten seconds respectively. Metadata/key
responses are limited to 1 MiB and browser token responses to 64 KiB, including
bodies without Content-Length. Non-success responses are rejected; provider
response bodies and request credentials are excluded from error messages.
Endpoints require HTTPS except explicit HTTP loopback, and reject user information
and fragments. Issuer queries are rejected as ambiguous identities.

Active session cookies are random 256-bit opaque identifiers (`s2.` plus 43
base64url characters). Provider tokens stay in a bounded process-local store,
so large valid tokens do not enlarge the cookie. Legacy encrypted-cookie helpers
remain available with their 3800-byte limit, but middleware rejects those cookies.
API middleware rejects any malformed or multiple Authorization headers before
considering cookies; it never falls back from an offered invalid credential.

### Shared browser login runtime

Construct `OidcBrowserSession::new(config)` once, then use `browser_layer()` for
UI/callback routes and `api_layer(audience)` for API routes. Cloned handles share
issuer discovery, key caches and pending login state. The original free builders
remain available, but each creates its own runtime; they do not share lifecycle
state with another independently constructed builder.

Login now uses independent random state and nonce values. Signed state carries a
strict ten-minute issue-time limit; a bounded runtime table (1024 pending logins)
also enforces monotonic expiry and single use. Callbacks consume the entry before
provider I/O and recheck its deadline after verification. Restart invalidates
pending logins, requiring a fresh navigation. Legacy state without issue time is
rejected. Duplicate matching cookie values are rejected across Cookie headers.
Callback failures clear state at the configured path, and callback/UI responses
use `Cache-Control: no-store`.

Code exchange requires Bearer token type and a signed ID token. Verification
checks issuer, client audience (independent of the API audience), expiry, issue
time, nonce and subject. Multiple audiences require this client as `azp`; any
present `azp` must match. Additional audiences must also be explicitly configured
in `trusted_id_token_audiences` (empty by default). A present `at_hash` must bind
the returned access token.
The access token must independently validate and have the same subject before
session issuance. `algorithms` defaults to RS256; use the constructor and set an
explicit asymmetric allowlist when another supported algorithm is needed.
These configuration fields require updating older struct literals.

Providers may omit refresh tokens: those sessions are bounded by the verified
access-token expiry. Refresh responses must validate under the original signed
identity before a new cookie is emitted. A refreshed ID token, when present,
must preserve issuer, subject, audience and authorized party; any nonce or
authentication time must match the original login. An unsuccessful proactive
refresh can retain a still-valid original access token without persisting the
failed response. Failed or cancelled attempts that might have reached the
provider are not automatically retried, because the refresh token may have rotated.

### Browser session lifetime and revocation

The runtime holds at most 1024 active sessions. Original `session_ttl` is enforced
with a monotonic deadline and never slides on requests or refresh. Cookie Max-Age
reflects the remaining original lifetime and refresh-token expiry. Dropping the
runtime or restarting the process invalidates sessions, even with the same key.
Multiple replicas require session affinity; this API provides no shared store.

Requests for one session serialize refresh. Short-lived refreshed tokens receive
a proactive-refresh cooldown of half their remaining lifetime, capped by the
configured skew and original session deadline. Expired access tokens bypass that
cooldown. Cookie-authenticated mutations and POST `/logout` require exactly one
Origin matching the registered callback origin or an explicitly configured
`trusted_browser_origins` entry, and reject cross-site fetches. The additional
allowlist defaults to empty, contains at most 32 exact canonical HTTPS or loopback
HTTP origins, and rejects credentials, paths, query/fragment and wildcards.
`OidcBrowserSession::permits_browser_origin` lets hosts validate their transport
configuration against this policy; hosts separately own CORS and Host checks.
Bearer API calls retain their independent validation path.

Middleware inserts both `OidcClaims` and `BrowserSessionAccess` into extensions.
Hosts must recheck `access.is_live()` before admitting queued mutations and select
on `access.invalidated()` for streams. `authenticate_cookie` provides the same
access handle to host adapters; trusted hosts can call `revoke_cookie` directly.
HTTP logout revokes the server record before clearing the cookie: copied cookies
and retained access handles immediately lose access. An optional issuer logout
redirect requests SSO logout; it does not prove provider revocation.

Migration: construct one `OidcBrowserSession` and derive all UI/API layers from
it. Independently constructed free builders cannot authenticate one another's
opaque cookies. Old encrypted cookies require a fresh login.

`examples/browser_session.rs` is a compiling host that composes the shared
runtime with UI and API routers. Its loopback listener and ephemeral session key
are for local development; a production host must supply deployment and key
lifecycle configuration.

This is bounded local validation, not complete browser deployment qualification.
Real-provider login/logout/refresh, TLS/proxy deployment, browser compatibility
and MSRV qualification remain necessary before relying on this flow in production.

## License

Apache-2.0 — same as `cli-framework`.
