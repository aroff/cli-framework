//! Tests for oidc_validation_layer middleware.

use std::time::{SystemTime, UNIX_EPOCH};

use axum::{response::IntoResponse, routing::get, Router};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use jsonwebtoken::Algorithm;
use rsa::{RsaPrivateKey, RsaPublicKey};
use serde_json::json;
use tower::{Layer, ServiceExt};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use cli_framework_oidc::server::{
    oidc_validation_layer, AudiencePolicy, OidcClaims, OidcValidationConfig,
};

// ── Key helpers ───────────────────────────────────────────────────────────────

struct TestKeyPair {
    pub public_key: RsaPublicKey,
    pub encoding_key: jsonwebtoken::EncodingKey,
    pub kid: String,
}

fn test_key_pair() -> TestKeyPair {
    let mut rng = rand_core::OsRng;
    let priv_key = RsaPrivateKey::new(&mut rng, 2048).expect("key gen");
    let pub_key = RsaPublicKey::from(&priv_key);
    let pem = rsa::pkcs8::EncodePrivateKey::to_pkcs8_pem(&priv_key, rsa::pkcs8::LineEnding::LF)
        .expect("pem");
    let encoding_key = jsonwebtoken::EncodingKey::from_rsa_pem(pem.as_bytes()).expect("enc key");
    TestKeyPair {
        public_key: pub_key,
        encoding_key,
        kid: "test-kid-1".to_string(),
    }
}

fn jwk_for_key(kp: &TestKeyPair) -> serde_json::Value {
    use rsa::traits::PublicKeyParts;
    let n = URL_SAFE_NO_PAD.encode(kp.public_key.n().to_bytes_be());
    let e = URL_SAFE_NO_PAD.encode(kp.public_key.e().to_bytes_be());
    json!({
        "kty": "RSA",
        "kid": kp.kid,
        "alg": "RS256",
        "use": "sig",
        "n": n,
        "e": e,
    })
}

fn jwk_for_key_no_kid(kp: &TestKeyPair) -> serde_json::Value {
    use rsa::traits::PublicKeyParts;
    let n = URL_SAFE_NO_PAD.encode(kp.public_key.n().to_bytes_be());
    let e = URL_SAFE_NO_PAD.encode(kp.public_key.e().to_bytes_be());
    json!({
        "kty": "RSA",
        "alg": "RS256",
        "use": "sig",
        "n": n,
        "e": e,
    })
}

fn mint_jwt(kp: &TestKeyPair, claims: serde_json::Value) -> String {
    let mut header = jsonwebtoken::Header::new(Algorithm::RS256);
    header.kid = Some(kp.kid.clone());
    jsonwebtoken::encode(&header, &claims, &kp.encoding_key).expect("encode")
}

fn mint_jwt_no_kid(kp: &TestKeyPair, claims: serde_json::Value) -> String {
    let header = jsonwebtoken::Header::new(Algorithm::RS256);
    jsonwebtoken::encode(&header, &claims, &kp.encoding_key).expect("encode")
}

fn mint_jwt_with_kid(kp: &TestKeyPair, claims: serde_json::Value, kid: &str) -> String {
    let mut header = jsonwebtoken::Header::new(Algorithm::RS256);
    header.kid = Some(kid.to_string());
    jsonwebtoken::encode(&header, &claims, &kp.encoding_key).expect("encode")
}

fn now_secs() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64
}

// ── Test app builder ──────────────────────────────────────────────────────────

fn make_cfg(server: &MockServer) -> OidcValidationConfig {
    OidcValidationConfig {
        issuer_url: server.uri(),
        audience: AudiencePolicy::Unchecked,
        jwks_uri: Some(format!("{}/jwks", server.uri())),
        algorithms: vec![Algorithm::RS256],
        jwks_ttl: std::time::Duration::from_secs(300),
        clock_skew: std::time::Duration::from_secs(60),
        min_refetch_interval: std::time::Duration::from_secs(60),
    }
}

async fn make_app(cfg: OidcValidationConfig) -> Router {
    let layer = oidc_validation_layer(cfg).expect("layer");

    async fn protected(claims: OidcClaims) -> impl IntoResponse {
        axum::Json(json!({
            "sub": claims.sub,
            "scopes": claims.scopes,
            "roles": claims.roles,
        }))
    }

    // The layer is typed as BoxCloneSyncServiceLayer<Router, ...> (Layer<Router>),
    // so we apply it by wrapping an inner Router, then mount via fallback_service.
    let inner = Router::new().route("/protected", get(protected));
    let protected_svc = layer.layer(inner);
    Router::new().fallback_service(protected_svc)
}

async fn setup_mock_jwks(server: &MockServer, kp: &TestKeyPair) {
    Mock::given(method("GET"))
        .and(path("/jwks"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "keys": [jwk_for_key(kp)]
        })))
        .mount(server)
        .await;
}

async fn setup_mock_discovery(server: &MockServer) {
    let base = server.uri();
    Mock::given(method("GET"))
        .and(path("/.well-known/openid-configuration"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "issuer": base,
            "token_endpoint": format!("{}/token", base),
            "jwks_uri": format!("{}/jwks", base),
        })))
        .mount(server)
        .await;
}

fn valid_claims(issuer: &str, aud: &str) -> serde_json::Value {
    let now = now_secs();
    json!({
        "sub": "user-123",
        "iss": issuer,
        "aud": aud,
        "exp": now + 3600,
        "iat": now,
        "scope": "read write",
    })
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn valid_token_returns_200() {
    let server = MockServer::start().await;
    setup_mock_discovery(&server).await;
    let kp = test_key_pair();
    setup_mock_jwks(&server, &kp).await;

    let mut cfg = make_cfg(&server);
    cfg.audience = AudiencePolicy::Require("my-app".into());

    let app = make_app(cfg).await;
    let token = mint_jwt(&kp, valid_claims(&server.uri(), "my-app"));

    let req = axum::http::Request::builder()
        .uri("/protected")
        .header("authorization", format!("Bearer {}", token))
        .body(axum::body::Body::empty())
        .unwrap();

    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), axum::http::StatusCode::OK);
}

#[tokio::test]
async fn missing_auth_header_returns_401_with_www_authenticate() {
    let server = MockServer::start().await;
    setup_mock_discovery(&server).await;
    let kp = test_key_pair();
    setup_mock_jwks(&server, &kp).await;

    let app = make_app(make_cfg(&server)).await;

    let req = axum::http::Request::builder()
        .uri("/protected")
        .body(axum::body::Body::empty())
        .unwrap();

    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), axum::http::StatusCode::UNAUTHORIZED);
    assert!(resp.headers().contains_key("www-authenticate"));
}

#[tokio::test]
async fn expired_token_returns_401() {
    let server = MockServer::start().await;
    setup_mock_discovery(&server).await;
    let kp = test_key_pair();
    setup_mock_jwks(&server, &kp).await;

    let mut cfg = make_cfg(&server);
    cfg.clock_skew = std::time::Duration::from_secs(0); // no skew tolerance

    let app = make_app(cfg).await;
    let now = now_secs();
    let expired_claims = json!({
        "sub": "user-123",
        "iss": server.uri(),
        "aud": "my-app",
        "exp": now - 3600, // expired 1 hour ago
        "iat": now - 7200,
    });
    let token = mint_jwt(&kp, expired_claims);

    let req = axum::http::Request::builder()
        .uri("/protected")
        .header("authorization", format!("Bearer {}", token))
        .body(axum::body::Body::empty())
        .unwrap();

    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), axum::http::StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn wrong_issuer_returns_401() {
    let server = MockServer::start().await;
    setup_mock_discovery(&server).await;
    let kp = test_key_pair();
    setup_mock_jwks(&server, &kp).await;

    let app = make_app(make_cfg(&server)).await;
    let now = now_secs();
    let wrong_iss_claims = json!({
        "sub": "user-123",
        "iss": "https://wrong-issuer.example.com",
        "aud": "my-app",
        "exp": now + 3600,
        "iat": now,
    });
    let token = mint_jwt(&kp, wrong_iss_claims);

    let req = axum::http::Request::builder()
        .uri("/protected")
        .header("authorization", format!("Bearer {}", token))
        .body(axum::body::Body::empty())
        .unwrap();

    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), axum::http::StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn wrong_audience_returns_401() {
    let server = MockServer::start().await;
    setup_mock_discovery(&server).await;
    let kp = test_key_pair();
    setup_mock_jwks(&server, &kp).await;

    let mut cfg = make_cfg(&server);
    cfg.audience = AudiencePolicy::Require("correct-audience".into());

    let app = make_app(cfg).await;
    let now = now_secs();
    let wrong_aud_claims = json!({
        "sub": "user-123",
        "iss": server.uri(),
        "aud": "wrong-audience",
        "exp": now + 3600,
        "iat": now,
    });
    let token = mint_jwt(&kp, wrong_aud_claims);

    let req = axum::http::Request::builder()
        .uri("/protected")
        .header("authorization", format!("Bearer {}", token))
        .body(axum::body::Body::empty())
        .unwrap();

    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), axum::http::StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn unknown_kid_returns_401() {
    let server = MockServer::start().await;
    setup_mock_discovery(&server).await;
    let kp = test_key_pair();
    setup_mock_jwks(&server, &kp).await;

    let app = make_app(make_cfg(&server)).await;
    let now = now_secs();
    let claims = json!({
        "sub": "user-123",
        "iss": server.uri(),
        "aud": "my-app",
        "exp": now + 3600,
        "iat": now,
    });
    // Use a kid that doesn't exist in the JWKS
    let token = mint_jwt_with_kid(&kp, claims, "nonexistent-kid");

    let req = axum::http::Request::builder()
        .uri("/protected")
        .header("authorization", format!("Bearer {}", token))
        .body(axum::body::Body::empty())
        .unwrap();

    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), axum::http::StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn no_kid_single_key_returns_200() {
    let server = MockServer::start().await;
    setup_mock_discovery(&server).await;
    let kp = test_key_pair();

    // JWKS with a single key that has no kid
    Mock::given(method("GET"))
        .and(path("/jwks"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "keys": [jwk_for_key_no_kid(&kp)]
        })))
        .mount(&server)
        .await;

    let app = make_app(make_cfg(&server)).await;
    let now = now_secs();
    let claims = json!({
        "sub": "user-456",
        "iss": server.uri(),
        "exp": now + 3600,
        "iat": now,
    });
    let token = mint_jwt_no_kid(&kp, claims);

    let req = axum::http::Request::builder()
        .uri("/protected")
        .header("authorization", format!("Bearer {}", token))
        .body(axum::body::Body::empty())
        .unwrap();

    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), axum::http::StatusCode::OK);
}

#[tokio::test]
async fn no_kid_multiple_keys_returns_401() {
    let server = MockServer::start().await;
    setup_mock_discovery(&server).await;
    let kp1 = test_key_pair();
    let kp2 = test_key_pair();

    // JWKS with two keys, neither has a kid
    Mock::given(method("GET"))
        .and(path("/jwks"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "keys": [jwk_for_key_no_kid(&kp1), jwk_for_key_no_kid(&kp2)]
        })))
        .mount(&server)
        .await;

    let app = make_app(make_cfg(&server)).await;
    let now = now_secs();
    let claims = json!({
        "sub": "user-789",
        "iss": server.uri(),
        "exp": now + 3600,
        "iat": now,
    });
    let token = mint_jwt_no_kid(&kp1, claims);

    let req = axum::http::Request::builder()
        .uri("/protected")
        .header("authorization", format!("Bearer {}", token))
        .body(axum::body::Body::empty())
        .unwrap();

    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), axum::http::StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn malformed_token_returns_401() {
    let server = MockServer::start().await;
    setup_mock_discovery(&server).await;
    let kp = test_key_pair();
    setup_mock_jwks(&server, &kp).await;

    let app = make_app(make_cfg(&server)).await;

    let req = axum::http::Request::builder()
        .uri("/protected")
        .header("authorization", "Bearer this.is.not.a.valid.jwt")
        .body(axum::body::Body::empty())
        .unwrap();

    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), axum::http::StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn scope_extracted_from_scope_field() {
    let server = MockServer::start().await;
    setup_mock_discovery(&server).await;
    let kp = test_key_pair();
    setup_mock_jwks(&server, &kp).await;

    let app = make_app(make_cfg(&server)).await;
    let now = now_secs();
    let claims = json!({
        "sub": "user-123",
        "iss": server.uri(),
        "exp": now + 3600,
        "iat": now,
        "scope": "read write admin",
    });
    let token = mint_jwt(&kp, claims);

    let req = axum::http::Request::builder()
        .uri("/protected")
        .header("authorization", format!("Bearer {}", token))
        .body(axum::body::Body::empty())
        .unwrap();

    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), axum::http::StatusCode::OK);

    let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    let scopes: Vec<&str> = json["scopes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap())
        .collect();
    assert!(scopes.contains(&"read"));
    assert!(scopes.contains(&"write"));
    assert!(scopes.contains(&"admin"));
}

#[tokio::test]
async fn scope_falls_back_to_scp_array() {
    let server = MockServer::start().await;
    setup_mock_discovery(&server).await;
    let kp = test_key_pair();
    setup_mock_jwks(&server, &kp).await;

    let app = make_app(make_cfg(&server)).await;
    let now = now_secs();
    let claims = json!({
        "sub": "user-123",
        "iss": server.uri(),
        "exp": now + 3600,
        "iat": now,
        "scp": ["profile", "email"],  // no "scope", only "scp"
    });
    let token = mint_jwt(&kp, claims);

    let req = axum::http::Request::builder()
        .uri("/protected")
        .header("authorization", format!("Bearer {}", token))
        .body(axum::body::Body::empty())
        .unwrap();

    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), axum::http::StatusCode::OK);

    let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    let scopes: Vec<&str> = json["scopes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap())
        .collect();
    assert!(scopes.contains(&"profile"));
    assert!(scopes.contains(&"email"));
}

#[tokio::test]
async fn roles_from_realm_access() {
    let server = MockServer::start().await;
    setup_mock_discovery(&server).await;
    let kp = test_key_pair();
    setup_mock_jwks(&server, &kp).await;

    let app = make_app(make_cfg(&server)).await;
    let now = now_secs();
    let claims = json!({
        "sub": "user-123",
        "iss": server.uri(),
        "exp": now + 3600,
        "iat": now,
        "realm_access": {
            "roles": ["admin", "viewer"]
        },
    });
    let token = mint_jwt(&kp, claims);

    let req = axum::http::Request::builder()
        .uri("/protected")
        .header("authorization", format!("Bearer {}", token))
        .body(axum::body::Body::empty())
        .unwrap();

    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), axum::http::StatusCode::OK);

    let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    let roles: Vec<&str> = json["roles"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap())
        .collect();
    assert!(roles.contains(&"admin"));
    assert!(roles.contains(&"viewer"));
}

#[tokio::test]
async fn jwks_unavailable_returns_503_with_retry_after() {
    let server = MockServer::start().await;

    // Discovery succeeds
    Mock::given(method("GET"))
        .and(path("/.well-known/openid-configuration"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "issuer": server.uri(),
            "token_endpoint": format!("{}/token", server.uri()),
            "jwks_uri": format!("{}/jwks", server.uri()),
        })))
        .mount(&server)
        .await;

    // JWKS returns 500
    Mock::given(method("GET"))
        .and(path("/jwks"))
        .respond_with(ResponseTemplate::new(500))
        .mount(&server)
        .await;

    let kp = test_key_pair();
    let app = make_app(make_cfg(&server)).await;
    let now = now_secs();
    let claims = json!({
        "sub": "user-123",
        "iss": server.uri(),
        "exp": now + 3600,
        "iat": now,
    });
    let token = mint_jwt(&kp, claims);

    let req = axum::http::Request::builder()
        .uri("/protected")
        .header("authorization", format!("Bearer {}", token))
        .body(axum::body::Body::empty())
        .unwrap();

    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), axum::http::StatusCode::SERVICE_UNAVAILABLE);
    assert!(
        resp.headers().contains_key("retry-after"),
        "503 must include retry-after header"
    );
}

#[tokio::test]
async fn no_layer_returns_500() {
    // Build an app WITHOUT the oidc layer but with an OidcClaims extractor
    async fn handler(claims: OidcClaims) -> impl IntoResponse {
        axum::Json(json!({"sub": claims.sub}))
    }

    let app = Router::new().route("/secret", get(handler));

    let req = axum::http::Request::builder()
        .uri("/secret")
        .header("authorization", "Bearer fake-token")
        .body(axum::body::Body::empty())
        .unwrap();

    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), axum::http::StatusCode::INTERNAL_SERVER_ERROR);
}
