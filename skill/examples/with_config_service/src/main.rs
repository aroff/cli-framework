//! Config-service example (spec 022)
//!
//! Demonstrates the whole read path end to end over a real HTTP socket:
//!
//! - An [`FsPolicyStore`] loaded from a bundle directory (no Postgres
//!   needed for this example — see spec 022 user story 30).
//! - [`ConfigServiceState::validate_at_startup`], which refuses to start if
//!   any stored policy fails manifest conformance.
//! - [`config_service_router`] mounted into an [`ApiServerBuilder`] the
//!   ordinary way (`.mount("/config", ...)`).
//! - The part spec 022 requires this example to prove, not merely describe
//!   in a doc comment: a real adapter from `cli-framework-oidc`'s
//!   [`OidcValidator`] to this crate's crate-local [`CallerIdentity`] trait
//!   — see [`OidcCallerIdentity`] below. To keep the example runnable with
//!   no external Keycloak instance, the "identity provider" here is a
//!   synthesized issuer: a locally generated signing key and an in-process
//!   mock HTTP server serving its JWKS, using the exact `test_support`
//!   helpers `cli-framework-oidc`'s own test suite uses (promoted
//!   specifically so downstream crates don't reinvent this).
//!
//! ```bash
//! cargo run --example with_config_service --features config-service
//! ```

use async_trait::async_trait;
use cli_framework::api::ApiServerBuilder;
use cli_framework::config::service::{
    config_service_router, CallerIdentity, ConfigServiceError, ConfigServiceState, FsPolicyStore,
    InMemoryUserConfigStore,
};
use cli_framework_oidc::server::{OidcValidationError, OidcValidator};
use cli_framework_oidc::test_support::{jwk_for_key, make_cfg, mint_jwt, now_secs, test_key_pair};
use serde_json::json;
use std::sync::Arc;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

/// The adapter spec 022 requires: converts `cli-framework-oidc`'s
/// [`OidcValidator`] into this crate's [`CallerIdentity`]. This is the
/// entire integration — a real application wires its own `OidcValidator`
/// (pointed at its real identity provider) the same way.
struct OidcCallerIdentity(OidcValidator);

#[async_trait]
impl CallerIdentity for OidcCallerIdentity {
    async fn authenticate(
        &self,
        authorization_header: Option<&str>,
    ) -> Result<serde_json::Value, ConfigServiceError> {
        match self.0.validate_authorization(authorization_header).await {
            Ok(claims) => Ok(claims.raw),
            Err(OidcValidationError::MissingToken) => Err(ConfigServiceError::MissingCredential),
            Err(e) => Err(ConfigServiceError::InvalidCredential(e.to_string())),
        }
    }
}

fn write_bundle(root: &std::path::Path) {
    let manifest = json!({
        "manifest_schema_version": 1,
        "app": "acme-desktop",
        "fields": [
            {"key": "greeting", "kind": "string", "scope": "machine"},
            {"key": "proxy_url", "kind": "url", "scope": "machine"}
        ]
    });
    std::fs::create_dir_all(root.join("manifests")).unwrap();
    std::fs::write(
        root.join("manifests/acme-desktop.json"),
        serde_json::to_string_pretty(&manifest).unwrap(),
    )
    .unwrap();

    std::fs::create_dir_all(root.join("policies/acme-desktop")).unwrap();
    std::fs::write(
        root.join("policies/acme-desktop/developers.toml"),
        r#"
version = 1
max_cache_age_secs = 3600
stale_action = "warn"

[enforced]
"proxy_url" = "https://proxy.acme.example.com"

[recommended]
"greeting" = "Welcome, developer"
"#,
    )
    .unwrap();

    std::fs::write(
        root.join("assignments.toml"),
        r#"
[acme-desktop]
[[acme-desktop.rules]]
claim_path = "realm_access.roles"
operator = "contains"
value = "developers"
profile = "developers"
"#,
    )
    .unwrap();
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    env_logger::init();

    // ── Synthesized identity provider ───────────────────────────────────
    let issuer = MockServer::start().await;
    let kp = test_key_pair();
    Mock::given(method("GET"))
        .and(path("/jwks"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "keys": [jwk_for_key(&kp)]
        })))
        .mount(&issuer)
        .await;

    let validator = OidcValidator::new(make_cfg(&issuer.uri()))?;
    let identity: Arc<dyn CallerIdentity> = Arc::new(OidcCallerIdentity(validator));

    // ── Config-service storage (bundle directory, no Postgres) ──────────
    let bundle_dir = tempfile::TempDir::new()?;
    write_bundle(bundle_dir.path());
    let policy_store = Arc::new(FsPolicyStore::load(bundle_dir.path())?);
    let user_config_store = Arc::new(InMemoryUserConfigStore::new());

    let state = ConfigServiceState::new(policy_store, user_config_store, identity);
    state
        .validate_at_startup()
        .await
        .map_err(|e| anyhow::anyhow!("config service refused to start: {e}"))?;

    // ── Mount into an ordinary ApiServerBuilder, like any other app ─────
    let server = ApiServerBuilder::new()
        .mount("/config", config_service_router(state))
        .version(cli_framework::api::ApiVersion {
            name: cli_framework::api::ApiVersionName::parse("v1")?,
            router: cli_framework::axum::Router::new(),
            stability: cli_framework::api::Stability::Stable,
            deprecation: None,
            #[cfg(feature = "api-swagger")]
            openapi: None,
        })
        .build();

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let addr = listener.local_addr()?;
    let router = server.into_router();
    tokio::spawn(async move {
        axum::serve(listener, router).await.unwrap();
    });

    // ── Mint a token for a "developers" identity and call the service ───
    let token = mint_jwt(
        &kp,
        json!({
            "sub": "alice",
            "iss": issuer.uri(),
            "aud": "acme-desktop",
            "exp": now_secs() + 300,
            "realm_access": {"roles": ["developers"]},
        }),
    );

    let client = reqwest::Client::new();
    let resp = client
        .get(format!("http://{addr}/config/v1/policy/acme-desktop"))
        .bearer_auth(&token)
        .send()
        .await?;
    println!("GET /config/v1/policy/acme-desktop -> {}", resp.status());
    println!("{}", resp.text().await?);

    // An unauthenticated request is rejected -- proving the router really
    // does authenticate itself, independent of anything ApiServerBuilder
    // would otherwise gate with `.auth(...)`.
    let unauth = client
        .get(format!("http://{addr}/config/v1/policy/acme-desktop"))
        .send()
        .await?;
    println!(
        "GET /config/v1/policy/acme-desktop (no token) -> {}",
        unauth.status()
    );

    Ok(())
}
