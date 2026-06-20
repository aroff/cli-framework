# ADR 0069: Auth layer — TokenProvider trait, auth feature, and cli-framework-oidc companion crate

- Date: 2026-06-16
- Status: Accepted (implemented & shipped in 0.5.8)
- Relates to: ADR 0060 (revisits its premise; does not reverse it — see Context); extended by ADR 0070 (JWKS-refetch single-flight on the `server` feature)

## Context

ADR 0060 removed an earlier `cli_framework::auth` module on the grounds that it was inert: the
framework runtime never called it, so exposing it misled consumers into thinking it was part of the
execution model. The decision told consumers to implement auth in their own application layer.

Two things have changed since then:

1. `ApiServerBuilder` now has an `auth(BoxCloneLayer)` mount point — a real framework hook that
   applies a tower middleware to all `/api/**` routes.
2. Internal applications are converging on Keycloak as the auth backend and need a standard,
   reusable auth path rather than each binary re-implementing OIDC.

The new auth layer has genuine integration points that the runtime calls. This is the distinction
from ADR 0060's inert helpers. Note this ADR does **not** reverse ADR 0060: removing the old inert
module was the right call and stands. This is a new, differently-shaped module filling the same
intent through hooks the runtime actually exercises — hence "relates to / revisits", not
"supersedes".

## Decision

### 1. Two-crate split

Auth lives in two crates:

- **`cli-framework`** (`auth` feature flag): the generic `TokenProvider` trait, `AccessToken`,
  `AuthenticatedHttpClient`, `Auth commands`, and the `AppContext::opt_token_provider()` accessor.
  No OIDC/OAuth2 dependencies.
- **`cli-framework-oidc`** (new companion crate, same workspace): split into two Cargo features
  matching its two independent halves:
  - **`client`** — the concrete `OidcClient` implementing `TokenProvider` plus all three OIDC
    flows. Depends on `cli-framework/auth`.
  - **`server`** — the `oidc_validation_layer()` tower middleware, `OidcClaims` axum extractor,
    and JWKS caching. Depends on `cli-framework/api-server`, **not** on `auth` — it validates
    incoming tokens and never constructs a `TokenProvider`.

  A consumer enables only the half it needs: a validate-only API server takes `server` alone and
  never compiles the `TokenProvider` trait or auth commands.

Consumers that use a static API key, a custom SSO, or no auth at all never take a dependency on
`cli-framework-oidc`. The crate boundary is the extensibility seam — Azure AD, GitHub OIDC, or
any future provider gets its own implementation without touching cli-framework core.

### 2. TokenProvider trait (in cli-framework, auth feature)

```rust
#[async_trait]
pub trait TokenProvider: Send + Sync {
    async fn token(&self) -> Result<AccessToken, AuthError>;
    async fn invalidate(&self);
    async fn login(&self) -> Result<(), AuthError> {
        Err(AuthError::NotSupported("login"))
    }
    async fn logout(&self) -> Result<(), AuthError> {
        Err(AuthError::NotSupported("logout"))
    }
}
```

- `token()` returns a currently-valid token, performing **non-interactive acquisition** when
  needed — a refresh-token exchange **or** a client-credentials grant — but **never an interactive
  flow** (Device Code / Auth Code + PKCE). Refresh/acquisition is the provider's internal concern;
  the framework never calls a separate `refresh()` method. It returns `AuthError::NotAuthenticated`
  only when a valid token cannot be obtained non-interactively (the sole remaining path is an
  interactive login) — so a client-credentials provider with an empty cache acquires directly
  rather than returning `NotAuthenticated`, while an interactive provider with an empty cache
  returns `NotAuthenticated`. Interactive acquisition is exclusively `auth login`'s job. This keeps
  `auth status`, `auth token`, and the 401-retry path **non-interactive** — they may hit the
  network, but no command spontaneously launches a browser/prompt.
- `invalidate()` is called by `AuthenticatedHttpClient` on a 401 response, after which `token()`
  is called once more and the request retried. If `token()` then returns `NotAuthenticated`, the
  client surfaces the error rather than launching a flow.
- `login()` and `logout()` default to `AuthError::NotSupported` — providers that don't support
  interactive login (static keys, env-var tokens) do not need to implement them.

### 3. Auth commands auto-registered by AppBuilder

When `AppBuilder::with_token_provider(Arc<dyn TokenProvider>)` is called, the framework
auto-registers four built-in commands under the `auth` group:

| Command | Behaviour |
|---|---|
| `auth login` | Calls `provider.login()`. If `NotSupported` → exit 1 with error message. |
| `auth logout` | Calls `provider.logout()`. If `NotSupported` → exit 1 with error message. |
| `auth status` | Calls `provider.token()` and reports expiry; on `NotAuthenticated` reports "not logged in; run `auth login`". Passive — never launches a flow. |
| `auth token` | Prints the raw bearer string; useful for `curl` debugging. Passive — surfaces `NotAuthenticated` rather than logging in. |

`auth login` returns exit 1 (not exit 0) when the provider does not support login. Silently
succeeding would leave a caller believing they are authenticated when they are not.

### 4. AppContext accessor

The framework exposes the wired provider via:

```rust
fn opt_token_provider(&self) -> Option<Arc<dyn TokenProvider>> { None }
```

This follows the established accessor pattern (`opt_registry`, `opt_global_args`) with one
deliberate difference: those return a borrowed `Option<&T>`, whereas `opt_token_provider` returns
an owned `Option<Arc<dyn TokenProvider>>`. The clone is cheap (an `Arc` bump) and lets a handler
move the provider into an `AuthenticatedHttpClient` that outlives the borrow of `&mut dyn
AppContext` — a borrowed reference could not. Handlers retrieve the provider and construct an
`AuthenticatedHttpClient` themselves — the framework does not pre-build or vend a shared HTTP
client instance, because handlers may need different base URLs or retry policies.

### 5. Server-side validation entirely in cli-framework-oidc

`ApiServerBuilder::auth(layer)` is already the generic mount; cli-framework adds nothing further
for the server side. `cli-framework-oidc` provides:

- `oidc_validation_layer(OidcValidationConfig) -> cli_framework::tower::util::BoxCloneLayer<axum::Router>`
  — the exact type `ApiServerBuilder::auth()` accepts (a tower `Layer`, not a bare middleware
  service). Note the path is `cli-framework`'s **shim** re-export (`src/tower.rs`), which aliases
  `tower::util::BoxCloneSyncServiceLayer<…>`; upstream `tower` has no `BoxCloneLayer`. The companion
  crate must name the shim path, not an upstream one.
- `OidcClaims` — typed axum extractor populated into the request extension map.
- JWKS caching: 5-minute TTL; on JWT signature validation failure with cached keys, JWKS is
  refetched once before returning 401 (transparent key-rotation handling); a short debounce
  prevents thundering-herd refetches under concurrent load.

### 6. Three OIDC flows in cli-framework-oidc v1

- **Device Code**: prints URL + code; user completes login on any device. Works headless/SSH.
- **Auth Code + PKCE**: opens browser on local machine; loopback server captures the code.
- **Client Credentials**: `client_id` + `client_secret` exchanged directly; for CI and service accounts.

Flow is a construction-time choice on `OidcClient`, not a runtime flag.

### 7. Token cache

The OIDC client persists access and refresh tokens to `<cache_dir>/oidc-token.json` (0600). The
cache directory is supplied explicitly at `OidcClient` construction time. When the config manager
(ADR 0067) ships, a convenience helper `OidcClient::cache_dir_from_config(ctx)` will read the
path from the config layer without requiring a structural change.

## Consequences

- Adds an `auth` feature to `cli-framework` with no breaking changes to existing consumers
  (feature is off by default).
- Adds a new `cli-framework-oidc` crate to the workspace.
- Consumers using the old removed `cli_framework::auth` (ADR 0060) will find a new, different
  `auth` module — incompatible in shape but filling the same intent.
- Internal applications (Keycloak backend) migrate to `cli-framework-oidc`; each application
  continues to own its `AppContext` impl and constructs `AuthenticatedHttpClient` in its handlers.
- Future providers (Azure AD, GitHub Actions OIDC) can be added to `cli-framework-oidc` or as
  additional companion crates without touching cli-framework core.
