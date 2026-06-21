//! Live end-to-end tests against a real Keycloak (or any OIDC provider).
//!
//! These are `#[ignore]`d so they never run in normal CI / `cargo test`. Run them
//! explicitly once the env vars below point at a reachable realm:
//!
//! ```bash
//! export KC_ISSUER_URL="https://keycloak.example.com/realms/my-realm"
//! export KC_CLIENT_ID="my-service"          # confidential client, Service Accounts ON
//! export KC_CLIENT_SECRET="..."             # from the client's Credentials tab
//! export KC_AUDIENCE="my-api"               # optional; omit → AudiencePolicy::Unchecked
//!
//! cargo test -p cli-framework-oidc --features client,server --test e2e_keycloak -- --ignored --nocapture
//! ```
//!
//! Reachability: the test host must reach the realm's discovery, token, and JWKS
//! endpoints over HTTPS (port-forward / ingress / VPN as needed).
//!
//! If the required vars are absent the tests **skip gracefully** (print + return)
//! rather than fail, so `cargo test -- --ignored` is safe without a configured IdP.
//!
//! This file is auto-discovered as a test target; the crate-level `cfg` makes it
//! compile to nothing unless both `client` and `server` features are enabled, so
//! no explicit `[[test]]` entry (with `required-features`) is needed.
#![cfg(all(feature = "client", feature = "server"))]

use std::time::Duration;

use axum::{response::IntoResponse, routing::get, Router};
use cli_framework::auth::TokenProvider;
use cli_framework_oidc::client::{OidcClient, OidcFlow, TokenAuthMethod};
use cli_framework_oidc::server::{
    oidc_validation_layer, AudiencePolicy, OidcClaims, OidcValidationConfig,
};
use secrecy::SecretString;
use serde_json::json;
use tower::Layer;

struct KcEnv {
    issuer: String,
    client_id: String,
    secret: String,
    audience: Option<String>,
}

/// Read the live-Keycloak config from the environment, or `None` (skip) if the
/// required vars are not set.
fn kc_env(test: &str) -> Option<KcEnv> {
    let get = |k: &str| std::env::var(k).ok().filter(|v| !v.is_empty());
    match (
        get("KC_ISSUER_URL"),
        get("KC_CLIENT_ID"),
        get("KC_CLIENT_SECRET"),
    ) {
        (Some(issuer), Some(client_id), Some(secret)) => Some(KcEnv {
            issuer,
            client_id,
            secret,
            audience: get("KC_AUDIENCE"),
        }),
        _ => {
            eprintln!(
                "[{test}] skipped: set KC_ISSUER_URL, KC_CLIENT_ID, KC_CLIENT_SECRET \
                 (and optionally KC_AUDIENCE) to run live-Keycloak e2e tests"
            );
            None
        }
    }
}

/// Build a Client Credentials client against the live realm, using a hermetic
/// temp cache so the test doesn't touch the user's real token cache.
fn client_credentials_client(env: &KcEnv, cache_dir: std::path::PathBuf) -> OidcClient {
    OidcClient::builder()
        .issuer_url(&env.issuer)
        .client_id(&env.client_id)
        .flow(OidcFlow::ClientCredentials {
            client_secret: SecretString::new(env.secret.clone()),
            token_auth: TokenAuthMethod::Post,
        })
        .cache_dir(cache_dir)
        .build()
        .expect("build OidcClient")
}

/// Client half: acquire an access token from the real token endpoint.
#[tokio::test]
#[ignore = "requires a live Keycloak; set KC_* env vars and run with --ignored"]
async fn client_credentials_acquires_token() {
    let Some(env) = kc_env("client_credentials_acquires_token") else {
        return;
    };
    let tmp = tempfile::TempDir::new().unwrap();
    let client = client_credentials_client(&env, tmp.path().to_path_buf());

    let token = client.token().await.expect("acquire token from Keycloak");
    assert!(
        !token.as_bearer().is_empty(),
        "Keycloak returned an empty access token"
    );
    eprintln!("acquired access token ({} bytes)", token.as_bearer().len());
}

/// Full round-trip: a token minted by the real Keycloak must pass our own
/// server-side validation layer (signature via live JWKS, issuer, audience).
#[tokio::test]
#[ignore = "requires a live Keycloak; set KC_* env vars and run with --ignored"]
async fn acquired_token_passes_server_validation() {
    let Some(env) = kc_env("acquired_token_passes_server_validation") else {
        return;
    };

    // 1) Acquire a real token (client half).
    let tmp = tempfile::TempDir::new().unwrap();
    let client = client_credentials_client(&env, tmp.path().to_path_buf());
    let token = client.token().await.expect("acquire token");

    // 2) Stand up the validation layer against the same realm (server half).
    let audience = match &env.audience {
        Some(aud) => AudiencePolicy::RequireAny(vec![aud.clone()]),
        None => AudiencePolicy::Unchecked,
    };
    let cfg = OidcValidationConfig::new(&env.issuer, audience);
    let layer = oidc_validation_layer(cfg).expect("build validation layer");

    async fn protected(claims: OidcClaims) -> impl IntoResponse {
        axum::Json(json!({ "sub": claims.sub, "scopes": claims.scopes }))
    }
    let inner = Router::new().route("/protected", get(protected));
    let app = Router::new().fallback_service(layer.layer(inner));

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });

    // 3) Call the protected route with the Keycloak-issued bearer.
    let resp = reqwest::Client::new()
        .get(format!("http://127.0.0.1:{port}/protected"))
        .header("authorization", format!("Bearer {}", token.as_bearer()))
        .timeout(Duration::from_secs(10))
        .send()
        .await
        .expect("request to protected route");

    assert_eq!(
        resp.status(),
        200,
        "Keycloak-issued token was rejected by the validation layer \
         (if 401 with invalid_audience, set KC_AUDIENCE or add a Keycloak audience mapper)"
    );
    let body: serde_json::Value = resp.json().await.unwrap();
    eprintln!("validated claims: {body}");
    assert!(
        body["sub"].as_str().is_some_and(|s| !s.is_empty()),
        "validated claims missing a subject"
    );
}
