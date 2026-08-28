//! End-to-end spec 021 coverage: a real `cli-framework-oidc` `OidcClient`
//! (Client Credentials flow) acquires a genuine signed JWT from a
//! synthesized wiremock issuer — using the `test-support` helpers promoted
//! out of `cli-framework-oidc/tests/server_validation.rs` — and that token
//! flows, as an ordinary `TokenProvider`, through `AuthenticatedHttpClient`
//! into a `PolicyClient` fetch and a `RoamingConfigClient` round trip against
//! a second wiremock server. The fetched `Policy` is then folded into
//! `resolve()` alongside local layers, end to end.
//!
//! This is the "this slice's own tests can mint real tokens with arbitrary
//! claims" requirement (spec 021 Testing Decisions) made concrete: nothing
//! here re-derives JWT minting or key generation.

use cli_framework::auth::{AuthenticatedHttpClient, TokenProvider};
use cli_framework::config::managed::{
    PolicyCache, PolicyClient, PolicyOutcome, RoamingConfigClient,
};
use cli_framework::config::manifest::IntoConfigManifest;
use cli_framework::config::resolution::{resolve, Layer, ResolutionInput};
use cli_framework::config::InMemoryBackend;
use cli_framework::http_retry::RetryableHttpClient;
use cli_framework::ConfigManifest;
use cli_framework_oidc::client::{OidcClient, OidcFlow, TokenAuthMethod};
use cli_framework_oidc::test_support::{jwk_for_key, mint_jwt, now_secs, test_key_pair};
use secrecy::SecretString;
use serde::{Deserialize, Serialize};
use serde_json::{json, Map};
use std::sync::Arc;
use wiremock::matchers::{header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

// ── A representative application config surface ─────────────────────────────

#[derive(Clone, Serialize, Deserialize, ConfigManifest)]
#[config_manifest(app = "myapp")]
struct AppConfig {
    #[manifest(scope = "org", protected)]
    compliance_endpoint: String,

    #[manifest(label = "Update check interval")]
    update_check_interval_secs: u32,

    #[manifest(scope = "user")]
    nickname: String,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            compliance_endpoint: "https://default-compliance.internal".to_string(),
            update_check_interval_secs: 3600,
            nickname: String::new(),
        }
    }
}

// ── Synthesized OIDC issuer (test-support) ──────────────────────────────────

/// Mount the discovery + token (+ jwks) routes on an already-started
/// `issuer` `MockServer` — the caller starts the server first so it can mint
/// a JWT whose `iss` claim matches the server's own (randomly-assigned-port)
/// URI before any route exists to serve it.
async fn mount_synthesized_issuer_routes(
    issuer: &MockServer,
    minted_jwt: &str,
    kp: &cli_framework_oidc::test_support::TestKeyPair,
) {
    let base = issuer.uri();

    Mock::given(method("GET"))
        .and(path("/.well-known/openid-configuration"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "issuer": base,
            "token_endpoint": format!("{base}/token"),
            "jwks_uri": format!("{base}/jwks"),
        })))
        .mount(issuer)
        .await;

    Mock::given(method("POST"))
        .and(path("/token"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "access_token": minted_jwt,
            "token_type": "Bearer",
            "expires_in": 3600,
        })))
        .mount(issuer)
        .await;

    // Not consumed by anything in this test (no server-side validation is in
    // scope here), but mounted for realism and so a future test extending
    // this one to validate the token has a working JWKS endpoint for free.
    Mock::given(method("GET"))
        .and(path("/jwks"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"keys": [jwk_for_key(kp)]})))
        .mount(issuer)
        .await;
}

fn client_credentials_token_provider(
    issuer_uri: &str,
    cache_dir: &std::path::Path,
) -> Arc<dyn TokenProvider> {
    let client = OidcClient::builder()
        .issuer_url(issuer_uri)
        .client_id("myapp-service")
        .flow(OidcFlow::ClientCredentials {
            client_secret: SecretString::new("service-secret".to_string()),
            token_auth: TokenAuthMethod::Post,
        })
        .cache_dir(cache_dir.to_path_buf())
        .build()
        .expect("valid OidcClient config");
    Arc::new(client)
}

#[tokio::test]
async fn real_oidc_client_credentials_token_flows_through_policy_and_roaming_fetch() {
    // 1. Mint a real, signed JWT via the promoted test-support helpers — an
    //    arbitrary-claims token, not a hard-coded opaque string. The issuer
    //    server is started first so its (randomly-assigned-port) URI is
    //    known before minting the `iss` claim.
    let kp = test_key_pair();
    let issuer = MockServer::start().await;
    let claims = json!({
        "sub": "service-account:myapp",
        "iss": issuer.uri(),
        "exp": now_secs() + 3600,
        "scope": "policy:read",
    });
    let minted_jwt = mint_jwt(&kp, claims);
    mount_synthesized_issuer_routes(&issuer, &minted_jwt, &kp).await;

    let cache_dir = tempfile::tempdir().unwrap();
    let provider = client_credentials_token_provider(&issuer.uri(), cache_dir.path());

    // Sanity: the provider really does hand back the exact minted JWT.
    let acquired = provider
        .token()
        .await
        .expect("client-credentials acquisition");
    assert_eq!(acquired.as_bearer(), minted_jwt);

    let http = Arc::new(AuthenticatedHttpClient::new(
        RetryableHttpClient::new(reqwest::Client::new()),
        provider,
    ));

    // 2. Policy server requires exactly this bearer token.
    let policy_server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/policy/myapp"))
        .and(header(
            "authorization",
            format!("Bearer {minted_jwt}").as_str(),
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "contract_version": 1,
            "app": "myapp",
            "profile": "developers",
            "policy_version": 4,
            "max_cache_age_secs": 3600,
            "stale_action": "warn",
            "enforced": { "compliance_endpoint": "https://org-mandated.example.com" },
            "recommended": { "update_check_interval_secs": 900 },
        })))
        .mount(&policy_server)
        .await;

    let policy_cache = PolicyCache::new(Arc::new(InMemoryBackend::new()));
    let policy_client = PolicyClient::new(http.clone(), policy_cache, policy_server.uri(), "myapp");

    let outcome = policy_client.fetch().await.expect("policy fetch");
    let policy = match outcome {
        PolicyOutcome::Fresh(policy) => policy,
        other => panic!("expected Fresh, got {other:?}"),
    };
    assert_eq!(policy.policy_version, 4);

    // 3. Fold the real fetched policy into resolution alongside local
    // layers, end to end.
    let manifest = AppConfig::config_manifest();
    let mut config_file = Map::new();
    config_file.insert(
        "compliance_endpoint".to_string(),
        json!("https://locally-edited-by-mistake.example.com"),
    );
    let input = ResolutionInput {
        recommended: policy.recommended.clone(),
        config_file,
        enforced: policy.enforced.clone(),
        ..Default::default()
    };
    let resolved = resolve(&manifest, &input);

    // org-scoped + enforced beats the local file entirely.
    assert_eq!(
        resolved.value("compliance_endpoint"),
        Some(&json!("https://org-mandated.example.com"))
    );
    assert!(resolved.provenance("compliance_endpoint").unwrap().locked);

    // recommended beats the built-in default (no config-file/env/flag set it).
    assert_eq!(
        resolved.value("update_check_interval_secs"),
        Some(&json!(900))
    );
    assert_eq!(
        resolved
            .provenance("update_check_interval_secs")
            .unwrap()
            .layer,
        Layer::Recommended
    );

    // untouched field still resolves to its manifest default.
    assert_eq!(resolved.value("nickname"), Some(&json!("")));
    assert_eq!(
        resolved.provenance("nickname").unwrap().layer,
        Layer::Default
    );

    // 4. Roaming user-config round trip over the SAME authenticated client
    // (proving TokenProvider reuse across both fetchers) — only the
    // `scope: user` field (`nickname`) may ever be sent.
    let roaming_server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/config/myapp"))
        .and(header(
            "authorization",
            format!("Bearer {minted_jwt}").as_str(),
        ))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("etag", "\"doc-v1\"")
                .set_body_json(json!({"nickname": "alice"})),
        )
        .mount(&roaming_server)
        .await;
    Mock::given(method("PUT"))
        .and(path("/v1/config/myapp"))
        .and(header("if-match", "\"doc-v1\""))
        .respond_with(ResponseTemplate::new(200))
        .mount(&roaming_server)
        .await;

    let roaming = RoamingConfigClient::new(http, roaming_server.uri(), "myapp");
    let doc = roaming.get().await.expect("roaming get");
    assert_eq!(doc.value.get("nickname"), Some(&json!("alice")));

    let mut write_doc = Map::new();
    write_doc.insert("nickname".to_string(), json!("bob"));
    write_doc.insert("compliance_endpoint".to_string(), json!("must-be-dropped"));
    roaming
        .put(&manifest, &write_doc, doc.etag.as_deref().unwrap())
        .await
        .expect("roaming put");

    let requests = roaming_server.received_requests().await.unwrap();
    let put_req = requests
        .iter()
        .find(|r| r.method.as_str() == "PUT")
        .unwrap();
    let sent_body: serde_json::Value = serde_json::from_slice(&put_req.body).unwrap();
    assert_eq!(sent_body, json!({"nickname": "bob"}));
}
