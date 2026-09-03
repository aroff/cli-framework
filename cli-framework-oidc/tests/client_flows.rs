//! Tests for OidcClient token flows (CC, device code, errors).

use std::str::FromStr;
use std::sync::{Arc, Mutex};

use secrecy::SecretString;
use tempfile::TempDir;
use wiremock::matchers::{body_string_contains, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use cli_framework::auth::{AuthFlowReporter, TokenProvider};
use cli_framework_oidc::client::{OidcClient, OidcFlow, TokenAuthMethod};

fn make_secret(s: &str) -> SecretString {
    SecretString::from_str(s).unwrap()
}

struct CapturingReporter {
    codes: Arc<Mutex<Vec<(String, String)>>>,
    messages: Arc<Mutex<Vec<String>>>,
}

impl CapturingReporter {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            codes: Arc::new(Mutex::new(vec![])),
            messages: Arc::new(Mutex::new(vec![])),
        })
    }
}

impl AuthFlowReporter for CapturingReporter {
    fn user_code(&self, verification_uri: &str, user_code: &str) {
        self.codes
            .lock()
            .unwrap()
            .push((verification_uri.to_string(), user_code.to_string()));
    }

    fn message(&self, line: &str) {
        self.messages.lock().unwrap().push(line.to_string());
    }
}

async fn setup_server_with_discovery(server: &MockServer) {
    let base = server.uri();
    Mock::given(method("GET"))
        .and(path("/.well-known/openid-configuration"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "issuer": base,
            "token_endpoint": format!("{}/token", base),
            "device_authorization_endpoint": format!("{}/device_authorization", base),
            "authorization_endpoint": format!("{}/authorize", base),
            "jwks_uri": format!("{}/jwks", base),
        })))
        .mount(server)
        .await;
}

#[tokio::test]
async fn cc_post_acquires_token() {
    let server = MockServer::start().await;
    setup_server_with_discovery(&server).await;

    Mock::given(method("POST"))
        .and(path("/token"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "access_token": "test-access-token",
            "token_type": "Bearer",
            "expires_in": 3600,
        })))
        .mount(&server)
        .await;

    let dir = TempDir::new().unwrap();
    let client = OidcClient::builder()
        .issuer_url(server.uri())
        .client_id("my-client")
        .flow(OidcFlow::ClientCredentials {
            client_secret: make_secret("s3cr3t"),
            token_auth: TokenAuthMethod::Post,
        })
        .cache_dir(dir.path().to_path_buf())
        .build()
        .unwrap();

    let token = client.token().await.expect("token");
    assert_eq!(token.as_bearer(), "test-access-token");
}

#[tokio::test]
async fn cc_basic_auth_acquires_token() {
    let server = MockServer::start().await;
    setup_server_with_discovery(&server).await;

    Mock::given(method("POST"))
        .and(path("/token"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "access_token": "basic-auth-token",
            "token_type": "Bearer",
            "expires_in": 3600,
        })))
        .mount(&server)
        .await;

    let dir = TempDir::new().unwrap();
    let client = OidcClient::builder()
        .issuer_url(server.uri())
        .client_id("my-client")
        .flow(OidcFlow::ClientCredentials {
            client_secret: make_secret("s3cr3t"),
            token_auth: TokenAuthMethod::Basic,
        })
        .cache_dir(dir.path().to_path_buf())
        .build()
        .unwrap();

    let token = client.token().await.expect("token");
    assert_eq!(token.as_bearer(), "basic-auth-token");
}

#[tokio::test]
async fn cc_token_cached_no_second_call() {
    let server = MockServer::start().await;
    setup_server_with_discovery(&server).await;

    Mock::given(method("POST"))
        .and(path("/token"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "access_token": "cached-token",
            "token_type": "Bearer",
            "expires_in": 3600,
        })))
        .expect(1) // Should only be called once
        .mount(&server)
        .await;

    let dir = TempDir::new().unwrap();
    let client = OidcClient::builder()
        .issuer_url(server.uri())
        .client_id("my-client")
        .flow(OidcFlow::ClientCredentials {
            client_secret: make_secret("s3cr3t"),
            token_auth: TokenAuthMethod::Post,
        })
        .cache_dir(dir.path().to_path_buf())
        .build()
        .unwrap();

    let t1 = client.token().await.expect("first token");
    let t2 = client.token().await.expect("second token");
    assert_eq!(t1.as_bearer(), t2.as_bearer());
}

#[tokio::test]
async fn cc_with_custom_scopes() {
    let server = MockServer::start().await;
    setup_server_with_discovery(&server).await;

    Mock::given(method("POST"))
        .and(path("/token"))
        .and(body_string_contains("scope=read+write"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "access_token": "scoped-token",
            "token_type": "Bearer",
            "expires_in": 3600,
        })))
        .mount(&server)
        .await;

    let dir = TempDir::new().unwrap();
    let client = OidcClient::builder()
        .issuer_url(server.uri())
        .client_id("my-client")
        .flow(OidcFlow::ClientCredentials {
            client_secret: make_secret("s3cr3t"),
            token_auth: TokenAuthMethod::Post,
        })
        .scopes(vec!["read".to_string(), "write".to_string()])
        .cache_dir(dir.path().to_path_buf())
        .build()
        .unwrap();

    let token = client.token().await.expect("scoped token");
    assert_eq!(token.as_bearer(), "scoped-token");
}

#[tokio::test]
async fn cc_token_error_returned() {
    let server = MockServer::start().await;
    setup_server_with_discovery(&server).await;

    Mock::given(method("POST"))
        .and(path("/token"))
        .respond_with(ResponseTemplate::new(400).set_body_json(serde_json::json!({
            "error": "invalid_client",
            "error_description": "bad credentials",
        })))
        .mount(&server)
        .await;

    let dir = TempDir::new().unwrap();
    let client = OidcClient::builder()
        .issuer_url(server.uri())
        .client_id("bad-client")
        .flow(OidcFlow::ClientCredentials {
            client_secret: make_secret("wrong"),
            token_auth: TokenAuthMethod::Post,
        })
        .cache_dir(dir.path().to_path_buf())
        .build()
        .unwrap();

    let err = client.token().await.unwrap_err();
    let msg = err.to_string();
    assert!(
        msg.contains("invalid_client") || msg.contains("authentication provider error"),
        "got: {msg}"
    );
}

#[tokio::test]
async fn device_code_login_calls_reporter() {
    let server = MockServer::start().await;
    setup_server_with_discovery(&server).await;

    Mock::given(method("POST"))
        .and(path("/device_authorization"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "device_code": "dev-abc",
            "user_code": "USER-CODE",
            "verification_uri": "https://device.example.com/activate",
            "expires_in": 300,
            "interval": 1,
        })))
        .mount(&server)
        .await;

    Mock::given(method("POST"))
        .and(path("/token"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "access_token": "device-token",
            "token_type": "Bearer",
            "expires_in": 3600,
        })))
        .mount(&server)
        .await;

    let reporter = CapturingReporter::new();
    let reporter_ref = reporter.clone();
    let dir = TempDir::new().unwrap();

    let client = OidcClient::builder()
        .issuer_url(server.uri())
        .client_id("my-client")
        .flow(OidcFlow::DeviceCode)
        .cache_dir(dir.path().to_path_buf())
        .reporter(reporter_ref)
        .build()
        .unwrap();

    client.login().await.expect("login");
    let codes = reporter.codes.lock().unwrap().clone();
    assert!(!codes.is_empty(), "reporter should have received user_code");
    assert_eq!(codes[0].1, "USER-CODE");
}

#[tokio::test]
async fn discovery_issuer_mismatch_is_error() {
    let server = MockServer::start().await;

    // Discovery returns wrong issuer
    Mock::given(method("GET"))
        .and(path("/.well-known/openid-configuration"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "issuer": "https://wrong-issuer.example.com",
            "token_endpoint": format!("{}/token", server.uri()),
            "jwks_uri": format!("{}/jwks", server.uri()),
        })))
        .mount(&server)
        .await;

    let dir = TempDir::new().unwrap();
    let client = OidcClient::builder()
        .issuer_url(server.uri())
        .client_id("my-client")
        .flow(OidcFlow::ClientCredentials {
            client_secret: make_secret("s3cr3t"),
            token_auth: TokenAuthMethod::Post,
        })
        .cache_dir(dir.path().to_path_buf())
        .build()
        .unwrap();

    let err = client.token().await.unwrap_err();
    let msg = err.to_string();
    assert!(
        msg.contains("mismatch")
            || msg.contains("issuer")
            || msg.contains("authentication provider"),
        "got: {msg}"
    );
}

// ── Device-code PKCE (RFC 8628 + RFC 7636) ──────────────────────────────────
//
// A provider that mandates PKCE rejects the device-authorization request when
// `code_challenge`/`code_challenge_method` are absent, which kills the flow
// before a user code is shown. These tests pin the wire contract: the challenge
// goes out with the device request, and the verifier that matches it goes out
// with the redemption.

/// Decode `application/x-www-form-urlencoded` request bodies into a lookup.
fn form_params(body: &[u8]) -> std::collections::HashMap<String, String> {
    url::form_urlencoded::parse(body)
        .map(|(k, v)| (k.into_owned(), v.into_owned()))
        .collect()
}

fn s256(verifier: &str) -> String {
    use base64::Engine as _;
    use sha2::{Digest, Sha256};
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()))
}

#[tokio::test]
async fn device_code_login_sends_pkce_challenge_and_matching_verifier() {
    let server = MockServer::start().await;
    setup_server_with_discovery(&server).await;

    Mock::given(method("POST"))
        .and(path("/device_authorization"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "device_code": "dev-abc",
            "user_code": "USER-CODE",
            "verification_uri": "https://device.example.com/activate",
            "expires_in": 300,
            "interval": 1,
        })))
        .mount(&server)
        .await;

    Mock::given(method("POST"))
        .and(path("/token"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "access_token": "device-token",
            "token_type": "Bearer",
            "expires_in": 3600,
        })))
        .mount(&server)
        .await;

    let dir = TempDir::new().unwrap();
    let client = OidcClient::builder()
        .issuer_url(server.uri())
        .client_id("my-client")
        .flow(OidcFlow::DeviceCode)
        .cache_dir(dir.path().to_path_buf())
        .reporter(CapturingReporter::new())
        .build()
        .unwrap();

    client.login().await.expect("login");

    let requests = server.received_requests().await.unwrap();
    let device_req = requests
        .iter()
        .find(|r| r.url.path() == "/device_authorization")
        .expect("device authorization request was sent");
    let device = form_params(&device_req.body);

    assert_eq!(
        device.get("code_challenge_method").map(String::as_str),
        Some("S256"),
        "device request must declare S256; got {device:?}"
    );
    let challenge = device
        .get("code_challenge")
        .expect("device request must carry a code_challenge");
    // 32 bytes of SHA-256 as unpadded base64url is exactly 43 chars.
    assert_eq!(challenge.len(), 43, "challenge: {challenge}");

    let token_req = requests
        .iter()
        .find(|r| r.url.path() == "/token")
        .expect("token request was sent");
    let token = form_params(&token_req.body);
    let verifier = token
        .get("code_verifier")
        .expect("device-code redemption must carry the code_verifier");

    assert_eq!(
        &s256(verifier),
        challenge,
        "the redeemed verifier must hash to the challenge that was sent"
    );
    assert_eq!(
        token.get("grant_type").map(String::as_str),
        Some("urn:ietf:params:oauth:grant-type:device_code")
    );
}

#[tokio::test]
async fn device_authorization_error_is_surfaced_and_not_polled() {
    let server = MockServer::start().await;
    setup_server_with_discovery(&server).await;

    // Exactly what Keycloak answers when the client mandates PKCE and the
    // request omits it.
    Mock::given(method("POST"))
        .and(path("/device_authorization"))
        .respond_with(ResponseTemplate::new(400).set_body_json(serde_json::json!({
            "error": "invalid_request",
            "error_description": "Missing parameter: code_challenge_method",
        })))
        .mount(&server)
        .await;

    Mock::given(method("POST"))
        .and(path("/token"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "access_token": "must-not-be-reached",
            "token_type": "Bearer",
            "expires_in": 3600,
        })))
        .mount(&server)
        .await;

    let dir = TempDir::new().unwrap();
    let client = OidcClient::builder()
        .issuer_url(server.uri())
        .client_id("my-client")
        .flow(OidcFlow::DeviceCode)
        .cache_dir(dir.path().to_path_buf())
        .reporter(CapturingReporter::new())
        .build()
        .unwrap();

    let err = client.login().await.expect_err("login must fail");
    let msg = err.to_string();
    assert!(
        msg.contains("invalid_request"),
        "error must name the provider error code; got: {msg}"
    );
    assert!(
        msg.contains("Missing parameter: code_challenge_method"),
        "error must carry the provider description; got: {msg}"
    );

    // The old code read every field with `unwrap_or("")` and went on to poll the
    // token endpoint with an empty device code, which reported a misleading
    // `invalid_grant`. Nothing may reach /token now.
    let polled = server
        .received_requests()
        .await
        .unwrap()
        .iter()
        .any(|r| r.url.path() == "/token");
    assert!(
        !polled,
        "must not poll the token endpoint after a rejection"
    );
}

#[test]
fn pkce_helpers_are_available_to_the_client_feature_and_are_random() {
    use cli_framework_oidc::pkce;

    let v1 = pkce::generate_verifier();
    let v2 = pkce::generate_verifier();
    assert_eq!(v1.len(), 43, "verifier should be 43 chars: {v1}");
    assert_ne!(v1, v2, "two verifiers must not collide");
    assert!(
        v1.chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_'),
        "verifier must be unpadded base64url: {v1}"
    );
    assert_eq!(pkce::derive_challenge(&v1), s256(&v1));

    let s1 = pkce::generate_state();
    let s2 = pkce::generate_state();
    assert_eq!(s1.len(), 32, "state should be 32 hex chars: {s1}");
    assert!(s1.chars().all(|c| c.is_ascii_hexdigit()), "state: {s1}");
    assert_ne!(s1, s2, "two states must not collide");
}
