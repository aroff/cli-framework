//! Authentication example
//!
//! Demonstrates:
//! - Implementing `TokenProvider` (stub reads MY_APP_TOKEN from env)
//! - Wiring it via `AppBuilder::with_token_provider`
//! - Auto-registered `auth login / logout / status / token` commands
//! - A command that calls a protected API using `AuthenticatedHttpClient`,
//!   with the bearer token injected automatically
//!
//! ---
//! For real Keycloak / OIDC integration swap the stub for `OidcClient`
//! from the `cli-framework-oidc` crate (feature `client`). The wiring is
//! identical — only the provider construction changes:
//!
//! ```toml
//! [dependencies]
//! cli-framework = { ..., features = ["auth"] }
//! cli-framework-oidc = { path = "../cli-framework-oidc", features = ["client"] }
//! dirs = "5"
//! ```
//!
//! ```rust
//! use cli_framework_oidc::client::{OidcClient, OidcFlow};
//!
//! let provider = Arc::new(
//!     OidcClient::builder()
//!         .issuer_url("https://keycloak.example.com/realms/my-realm")
//!         .client_id("my-cli")
//!         .flow(OidcFlow::DeviceCode)          // or AuthCodePkce / ClientCredentials
//!         .cache_dir(dirs::cache_dir().unwrap().join("my-app"))
//!         .build()?
//! );
//! // rest of AppBuilder chain is identical
//! ```
//!
//! Keycloak client (Device Code / PKCE): public client, Standard Flow on,
//! Device Authorization Grant on, redirect URI http://127.0.0.1:8765/callback.
//!
//! ---
//!
//! ```bash
//! cargo run --example with_auth --features auth
//!
//! # With a real token provider: log in (Device Code prints URL + user_code)
//! with-auth auth login
//!
//! # Check status
//! with-auth auth status
//! with-auth auth status --json
//!
//! # Print raw bearer to stdout — useful for curl
//! with-auth auth token
//! curl -H "Authorization: Bearer $(with-auth auth token)" https://api.example.com/things
//!
//! # Authenticated command
//! MY_APP_TOKEN=my-bearer-value with-auth list-things
//!
//! # Logout
//! with-auth auth logout
//! ```

use cli_framework::auth::{
    AccessToken, AuthError, AuthenticatedHttpClient, TokenProvider, TokenStatus,
};
use cli_framework::http_retry::RetryableHttpClient;
use cli_framework::prelude::*;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, SystemTime};

// ── AppContext ────────────────────────────────────────────────────────────────

struct AppCtx;
impl AppContext for AppCtx {}

// ── Stub TokenProvider ────────────────────────────────────────────────────────
//
// Reads MY_APP_TOKEN from the environment. Replace this with OidcClient for
// real Keycloak/OIDC integration (see module-level doc comment above).

struct EnvTokenProvider;

#[async_trait::async_trait]
impl TokenProvider for EnvTokenProvider {
    async fn token(&self) -> Result<AccessToken, AuthError> {
        match std::env::var("MY_APP_TOKEN") {
            Ok(t) if !t.is_empty() => Ok(AccessToken::new(
                t,
                Some(SystemTime::now() + Duration::from_secs(3600)),
            )),
            _ => Err(AuthError::NotAuthenticated),
        }
    }

    async fn invalidate(&self) {
        // No local state to clear for env-var tokens.
    }

    async fn peek(&self) -> Option<TokenStatus> {
        let logged_in = std::env::var("MY_APP_TOKEN")
            .map(|v| !v.is_empty())
            .unwrap_or(false);
        Some(TokenStatus {
            logged_in,
            expires_at: None,
        })
    }

    // login/logout use the default NotSupported impl — env vars don't have a
    // login flow. OidcClient overrides these with the real interactive flow.
}

// ── Commands ──────────────────────────────────────────────────────────────────

fn list_things_cmd(api: Arc<AuthenticatedHttpClient>) -> anyhow::Result<Command> {
    // The execute closure captures `api` via Arc — this is the idiomatic way to
    // share an AuthenticatedHttpClient across commands. The AppContext trait
    // object doesn't expose crate-specific fields, so capture at build time.
    Ok(Command {
        id: Arc::from("list-things"),
        spec: Arc::new(CommandSpec {
            summary: "List things from the protected API",
            syntax: Some("list-things"),
            category: Some("things"),
            ..Default::default()
        }),
        validator: None,
        expose_mcp: true,
        expose_chat: true,
        meta: None,
        visibility: None,
        execute: Arc::new(move |_ctx, _args: HashMap<String, ArgValue>| {
            let api = api.clone();
            Box::pin(async move {
                // AuthenticatedHttpClient injects Authorization: Bearer automatically.
                // On 401 it calls invalidate() (nulls the access token, keeps the
                // refresh token), re-calls token() (tries the refresh grant), and
                // retries the request once.
                let resp = api
                    .get("https://api.example.com/things")
                    .await?
                    .error_for_status()?;

                let body: serde_json::Value = resp.json().await?;
                println!("{}", serde_json::to_string_pretty(&body)?);
                Ok(())
            })
        }),
    })
}

// ── main ──────────────────────────────────────────────────────────────────────

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Build the provider. Swap this Arc::new(EnvTokenProvider) for
    // Arc::new(OidcClient::builder()...build()?) to use Keycloak.
    let provider: Arc<dyn TokenProvider> = Arc::new(EnvTokenProvider);

    // AuthenticatedHttpClient wraps RetryableHttpClient.
    // Share a single instance across all commands that call the API.
    let api = Arc::new(AuthenticatedHttpClient::new(
        RetryableHttpClient::new(reqwest::Client::new()),
        provider.clone(),
    ));

    let mut app = AppBuilder::new()
        .with_version("with-auth", "0.1.0")
        // Registering the provider auto-adds:
        //   auth login / auth logout / auth status / auth token
        // These are never exposed as MCP tools or chat tools.
        .with_token_provider(provider)
        .register_command(list_things_cmd(api)?)?
        .build(AppCtx)?;

    app.run().await
}
