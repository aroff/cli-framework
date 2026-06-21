//! Real OIDC / Keycloak CLI example.
//!
//! A runnable, copyable CLI that authenticates against a live OIDC provider
//! (Keycloak, Azure AD, …) using `cli-framework-oidc`. Unlike the stub-based
//! `skill/examples/with_auth`, this wires the real `OidcClient`.
//!
//! It demonstrates the ergonomic path:
//! - `OidcClientBuilder::from_env("KC")` — config from env, no hand-wiring
//! - `OidcFlow::auto_interactive()` — PKCE on a desktop, Device Code over SSH
//!   (selected automatically unless `KC_FLOW` is set)
//! - a default cache dir derived from `.app_name(..)` (no `dirs` boilerplate)
//! - the auto-registered `auth login / logout / status / token` commands
//! - a `whoami` command that calls Keycloak's userinfo endpoint with the token
//!
//! ## Configure (point it at your cluster's Keycloak)
//!
//! ```bash
//! export KC_ISSUER_URL="https://keycloak.example.com/realms/my-realm"
//! export KC_CLIENT_ID="my-cli"
//! # Interactive (human login): leave KC_CLIENT_SECRET unset.
//! #   Keycloak client: public, Standard Flow ON, Device Authorization Grant ON,
//! #   Valid Redirect URI http://127.0.0.1:8765/callback
//! # Machine-to-machine: set the secret → Client Credentials is selected.
//! #   export KC_CLIENT_SECRET="..."   # confidential client, Service Accounts ON
//! # Optional overrides:
//! #   export KC_FLOW="device"|"pkce"|"client-credentials"|"auto"
//! #   export KC_SCOPES="openid profile email"
//! ```
//!
//! ## Run
//!
//! ```bash
//! cargo run -p cli-framework-oidc --example keycloak_cli --features client -- auth login
//! cargo run -p cli-framework-oidc --example keycloak_cli --features client -- auth status
//! cargo run -p cli-framework-oidc --example keycloak_cli --features client -- whoami
//! cargo run -p cli-framework-oidc --example keycloak_cli --features client -- auth token
//! cargo run -p cli-framework-oidc --example keycloak_cli --features client -- auth logout
//! ```

use std::collections::HashMap;
use std::sync::Arc;

use cli_framework::auth::{AuthenticatedHttpClient, TokenProvider};
use cli_framework::http_retry::RetryableHttpClient;
use cli_framework::prelude::*;
use cli_framework_oidc::client::OidcClientBuilder;

struct AppCtx;
impl AppContext for AppCtx {}

/// `whoami` — proves the acquired token works by calling Keycloak's userinfo
/// endpoint (`{issuer}/protocol/openid-connect/userinfo`) with the bearer.
fn whoami_cmd(api: Arc<AuthenticatedHttpClient>, issuer: String) -> anyhow::Result<Command> {
    Ok(Command {
        id: Arc::from("whoami"),
        spec: Arc::new(CommandSpec {
            summary: "Show the current identity via the OIDC userinfo endpoint",
            syntax: Some("whoami"),
            category: Some("auth"),
            ..Default::default()
        }),
        validator: None,
        expose_mcp: false,
        expose_chat: false,
        meta: None,
        visibility: None,
        execute: Arc::new(move |_ctx, _args: HashMap<String, ArgValue>| {
            let api = api.clone();
            let url = format!(
                "{}/protocol/openid-connect/userinfo",
                issuer.trim_end_matches('/')
            );
            Box::pin(async move {
                // AuthenticatedHttpClient injects `Authorization: Bearer <token>`
                // and, on a 401, invalidates + refreshes + retries once.
                let resp = api.get(&url).await?.error_for_status()?;
                let body: serde_json::Value = resp.json().await?;
                println!("{}", serde_json::to_string_pretty(&body)?);
                Ok(())
            })
        }),
    })
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Config straight from the environment. Explicit KC_FLOW wins; otherwise a
    // present KC_CLIENT_SECRET selects Client Credentials and its absence picks
    // an interactive flow suited to the session.
    let issuer = std::env::var("KC_ISSUER_URL").unwrap_or_default();
    let builder = match OidcClientBuilder::from_env("KC") {
        Ok(b) => b,
        Err(e) => {
            eprintln!(
                "Missing OIDC config: {e}\n\
                 Set at least KC_ISSUER_URL and KC_CLIENT_ID (see the example header)."
            );
            std::process::exit(2);
        }
    };

    // With no KC_FLOW and no secret, from_env() already selected an interactive
    // flow via OidcFlow::auto_interactive() (PKCE on a desktop, Device Code over
    // SSH). The cache dir is derived from app_name — no `dirs` dependency needed.
    let oidc = builder.app_name("keycloak-cli").build()?;
    let provider: Arc<dyn TokenProvider> = Arc::new(oidc);

    let api = Arc::new(AuthenticatedHttpClient::new(
        RetryableHttpClient::new(reqwest::Client::new()),
        provider.clone(),
    ));

    let mut app = AppBuilder::new()
        .with_version("keycloak-cli", "0.1.0")
        .with_token_provider(provider) // auto-registers auth login/logout/status/token
        .register_command(whoami_cmd(api, issuer)?)?
        .build(AppCtx)?;

    app.run().await
}
