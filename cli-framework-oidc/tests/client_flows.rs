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
