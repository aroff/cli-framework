//! Tests for oidc_validation_layer middleware.

use axum::{response::IntoResponse, routing::get, Router};
use jsonwebtoken::Algorithm;
use serde_json::json;
use tower::{Layer, ServiceExt};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use cli_framework_oidc::server::{
    oidc_validation_layer, AudiencePolicy, OidcClaims, OidcValidationConfig,
};

// ── Key/JWT/config helpers ──────────────────────────────────────────────────
//
// Promoted to `cli_framework_oidc::test_support` (spec 021 testing
// decisions) — this file no longer keeps a private copy. Keys are P-256 /
// ES256 generated via `rcgen` (backed by ring), avoiding the `rsa` crate
// (RUSTSEC-2023-0071, no upstream fix).
use cli_framework_oidc::test_support::{
    jwk_for_key, jwk_for_key_no_kid, mint_jwt, mint_jwt_no_kid, mint_jwt_with_kid, now_secs,
    test_key_pair, TestKeyPair,
};

// ── Test app builder ──────────────────────────────────────────────────────────

/// Thin, file-local adapter over the shared `test_support::make_cfg` (which
/// takes a bare issuer URI, since the promoted helpers live in the library
/// crate and cannot depend on `wiremock`): every call site in this file
/// already has a `&MockServer` in hand.
fn make_cfg(server: &MockServer) -> OidcValidationConfig {
    cli_framework_oidc::test_support::make_cfg(&server.uri())
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

// ── D1: AudiencePolicy::RequireAny (Keycloak aud is an array) ─────────────────

#[tokio::test]
async fn require_any_accepts_token_matching_one_of_several() {
    let server = MockServer::start().await;
    setup_mock_discovery(&server).await;
    let kp = test_key_pair();
    setup_mock_jwks(&server, &kp).await;

    let mut cfg = make_cfg(&server);
    cfg.audience = AudiencePolicy::RequireAny(vec!["api-two".into(), "api-one".into()]);

    let app = make_app(cfg).await;
    let token = mint_jwt(&kp, valid_claims(&server.uri(), "api-one"));

    let req = axum::http::Request::builder()
        .uri("/protected")
        .header("authorization", format!("Bearer {}", token))
        .body(axum::body::Body::empty())
        .unwrap();

    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), axum::http::StatusCode::OK);
}

#[tokio::test]
async fn require_any_rejects_token_matching_none() {
    let server = MockServer::start().await;
    setup_mock_discovery(&server).await;
    let kp = test_key_pair();
    setup_mock_jwks(&server, &kp).await;

    let mut cfg = make_cfg(&server);
    cfg.audience = AudiencePolicy::RequireAny(vec!["api-two".into(), "api-three".into()]);

    let app = make_app(cfg).await;
    let token = mint_jwt(&kp, valid_claims(&server.uri(), "api-one"));

    let req = axum::http::Request::builder()
        .uri("/protected")
        .header("authorization", format!("Bearer {}", token))
        .body(axum::body::Body::empty())
        .unwrap();

    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), axum::http::StatusCode::UNAUTHORIZED);
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

// ── C1: unknown-kid forced refetch ───────────────────────────────────────────
//
// When a JWT presents a kid that isn't in the cached JWKS, the layer must
// attempt one forced refetch (outside min_refetch_interval). If the refetch
// succeeds and the new JWKS contains the key, the request must succeed (200).
// A second unknown-kid request within min_refetch_interval must NOT trigger
// another fetch — it gets an immediate 401.

#[tokio::test]
async fn unknown_kid_triggers_refetch_and_succeeds_when_new_key_present() {
    // Strategy: prime the cache with kp_old via request 1, then send request 2
    // signed with kp_new. The layer must detect unknown kid, do a forced refetch
    // (which now returns both keys), and return 200.
    let mock = MockServer::start().await;
    let kp_old = test_key_pair();
    // kp_new needs a DISTINCT kid so filter_keys returns UnknownKid on the first pass.
    let kp_new = TestKeyPair {
        kid: "test-kid-2".to_string(),
        ..test_key_pair()
    };

    // JWKS fetch 1: only old key (primes the cache)
    Mock::given(method("GET"))
        .and(path("/jwks"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "keys": [jwk_for_key(&kp_old)]
        })))
        .up_to_n_times(1)
        .mount(&mock)
        .await;

    // JWKS fetch 2+: both keys (returned on the kid-miss forced refetch)
    Mock::given(method("GET"))
        .and(path("/jwks"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "keys": [jwk_for_key(&kp_old), jwk_for_key(&kp_new)]
        })))
        .mount(&mock)
        .await;

    let cfg = OidcValidationConfig {
        min_refetch_interval: std::time::Duration::from_secs(0), // no rate-limit for test
        ..make_cfg(&mock)
    };
    let issuer = cfg.issuer_url.clone();
    let layer = oidc_validation_layer(cfg).expect("layer");

    async fn protected(claims: OidcClaims) -> impl IntoResponse {
        axum::Json(json!({"sub": claims.sub}))
    }

    let inner = Router::new().route("/protected", get(protected));
    let protected_svc = layer.layer(inner);
    let app = Router::new().fallback_service(protected_svc);

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });

    let base_url = format!("http://127.0.0.1:{}", port);
    let client = reqwest::Client::new();
    let now = now_secs();

    // Request 1: old key → primes cache with kp_old only → 200
    let t1 = mint_jwt(
        &kp_old,
        json!({ "sub": "u", "iss": issuer, "exp": now + 3600i64 }),
    );
    let r1 = client
        .get(format!("{}/protected", base_url))
        .header("authorization", format!("Bearer {}", t1))
        .send()
        .await
        .unwrap();
    assert_eq!(r1.status(), 200, "request 1 with kp_old must succeed");

    // Request 2: new key → unknown kid → forced refetch → kp_old+kp_new in JWKS → 200
    let t2 = mint_jwt(
        &kp_new,
        json!({ "sub": "u", "iss": issuer, "exp": now + 3600i64 }),
    );
    let r2 = client
        .get(format!("{}/protected", base_url))
        .header("authorization", format!("Bearer {}", t2))
        .send()
        .await
        .unwrap();
    assert_eq!(
        r2.status(),
        200,
        "request 2 with kp_new must succeed after forced kid-miss refetch"
    );
}

// ── B5: discovered insecure JWKS URI → 503 ───────────────────────────────────

#[tokio::test]
async fn discovered_insecure_jwks_uri_returns_503() {
    let mock = MockServer::start().await;
    let base = mock.uri();

    // Discovery doc has valid issuer but points JWKS at plain http (non-loopback)
    Mock::given(method("GET"))
        .and(path("/.well-known/openid-configuration"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "issuer": base,
            "jwks_uri": "http://public.example.com/jwks", // insecure non-loopback
        })))
        .mount(&mock)
        .await;

    let cfg = OidcValidationConfig {
        issuer_url: base.clone(),
        audience: AudiencePolicy::Unchecked,
        jwks_uri: None, // force discovery path
        algorithms: vec![Algorithm::ES256],
        jwks_ttl: std::time::Duration::from_secs(300),
        clock_skew: std::time::Duration::from_secs(0),
        min_refetch_interval: std::time::Duration::from_secs(0),
    };
    let app = make_app(cfg).await;

    let kp = test_key_pair();
    let now = now_secs();
    let claims = json!({ "sub": "u", "iss": base, "exp": now + 3600i64 });
    let token = mint_jwt(&kp, claims);

    let req = axum::http::Request::builder()
        .uri("/protected")
        .header("authorization", format!("Bearer {}", token))
        .body(axum::body::Body::empty())
        .unwrap();

    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(
        resp.status(),
        503,
        "discovered insecure JWKS URI must return 503"
    );
}

// ── B4: discovery issuer mismatch → 503 ──────────────────────────────────────

#[tokio::test]
async fn discovery_issuer_mismatch_returns_503() {
    let mock = MockServer::start().await;
    let kp = test_key_pair();
    setup_mock_jwks(&mock, &kp).await;

    let base = mock.uri();
    // Discovery doc claims a DIFFERENT issuer
    Mock::given(method("GET"))
        .and(path("/.well-known/openid-configuration"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "issuer": "https://attacker.example.com/realm",
            "jwks_uri": format!("{}/jwks", base),
        })))
        .mount(&mock)
        .await;

    // cfg.issuer_url = base (the mock URL); discovery doc says attacker's issuer → mismatch
    let cfg = OidcValidationConfig {
        issuer_url: base.clone(),
        audience: AudiencePolicy::Unchecked,
        jwks_uri: None, // force discovery path
        algorithms: vec![Algorithm::ES256],
        jwks_ttl: std::time::Duration::from_secs(300),
        clock_skew: std::time::Duration::from_secs(0),
        min_refetch_interval: std::time::Duration::from_secs(0),
    };
    let app = make_app(cfg).await;

    let now = now_secs();
    let claims = json!({ "sub": "u", "iss": base, "exp": now + 3600i64 });
    let token = mint_jwt(&kp, claims);

    let req = axum::http::Request::builder()
        .uri("/protected")
        .header("authorization", format!("Bearer {}", token))
        .body(axum::body::Body::empty())
        .unwrap();

    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(
        resp.status(),
        503,
        "discovery issuer mismatch must return 503 (JWKS unavailable), not 401 or 200"
    );
}

// ── B2/B3: missing required claims → malformed_token ─────────────────────────

#[tokio::test]
async fn missing_exp_returns_401_malformed_token() {
    let mock = MockServer::start().await;
    let kp = test_key_pair();
    setup_mock_jwks(&mock, &kp).await;

    let cfg = make_cfg(&mock);
    let issuer = cfg.issuer_url.clone();
    let app = make_app(cfg).await;

    // JWT without exp claim
    let claims = json!({ "sub": "u", "iss": issuer });
    let token = mint_jwt(&kp, claims);

    let req = axum::http::Request::builder()
        .uri("/protected")
        .header("authorization", format!("Bearer {}", token))
        .body(axum::body::Body::empty())
        .unwrap();

    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), 401);
    let www = resp
        .headers()
        .get("www-authenticate")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    assert_eq!(
        extract_error_description(www),
        Some("malformed_token"),
        "missing exp must produce malformed_token; got: {:?}",
        www
    );
}

#[tokio::test]
async fn missing_sub_returns_401_malformed_token() {
    let mock = MockServer::start().await;
    let kp = test_key_pair();
    setup_mock_jwks(&mock, &kp).await;

    let cfg = make_cfg(&mock);
    let issuer = cfg.issuer_url.clone();
    let app = make_app(cfg).await;

    let now = now_secs();
    // JWT without sub claim
    let claims = json!({ "iss": issuer, "exp": now + 3600i64 });
    let token = mint_jwt(&kp, claims);

    let req = axum::http::Request::builder()
        .uri("/protected")
        .header("authorization", format!("Bearer {}", token))
        .body(axum::body::Body::empty())
        .unwrap();

    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), 401);
    let www = resp
        .headers()
        .get("www-authenticate")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    assert_eq!(
        extract_error_description(www),
        Some("malformed_token"),
        "missing sub must produce malformed_token; got: {:?}",
        www
    );
}

// ── B1: error_description closed set ─────────────────────────────────────────

fn extract_error_description(www: &str) -> Option<&str> {
    // e.g. Bearer error="invalid_token", error_description="expired"
    let prefix = "error_description=\"";
    let start = www.find(prefix)? + prefix.len();
    let end = www[start..].find('"')? + start;
    Some(&www[start..end])
}

#[tokio::test]
async fn error_description_expired_for_expired_token() {
    let mock = MockServer::start().await;
    let kp = test_key_pair();
    setup_mock_jwks(&mock, &kp).await;

    let cfg = make_cfg(&mock);
    let issuer = cfg.issuer_url.clone();
    let app = make_app(cfg).await;

    let now = now_secs();
    let claims = json!({
        "sub": "u", "iss": issuer,
        "exp": now - 200i64, // expired, well outside clock_skew=60
        "iat": now - 300i64,
    });
    let token = mint_jwt(&kp, claims);

    let req = axum::http::Request::builder()
        .uri("/protected")
        .header("authorization", format!("Bearer {}", token))
        .body(axum::body::Body::empty())
        .unwrap();

    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), 401);
    let www = resp
        .headers()
        .get("www-authenticate")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    assert_eq!(
        extract_error_description(www),
        Some("expired"),
        "expired token must produce error_description=expired; got www: {:?}",
        www
    );
}

#[tokio::test]
async fn error_description_invalid_signature_for_wrong_key() {
    let mock = MockServer::start().await;
    let kp = test_key_pair();
    setup_mock_jwks(&mock, &kp).await;

    let cfg = make_cfg(&mock);
    let issuer = cfg.issuer_url.clone();
    let app = make_app(cfg).await;

    // Sign with a DIFFERENT key pair — verification must fail with invalid_signature
    let other_kp = test_key_pair();
    let now = now_secs();
    let claims = json!({ "sub": "u", "iss": issuer, "exp": now + 3600i64 });
    // mint_jwt uses kp but we need other_kp — mint manually
    let other_token = mint_jwt_with_kid(&other_kp, claims, &kp.kid);

    let req = axum::http::Request::builder()
        .uri("/protected")
        .header("authorization", format!("Bearer {}", other_token))
        .body(axum::body::Body::empty())
        .unwrap();

    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), 401);
    let www = resp
        .headers()
        .get("www-authenticate")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    assert_eq!(
        extract_error_description(www),
        Some("invalid_signature"),
        "wrong-key token must produce error_description=invalid_signature; got: {:?}",
        www
    );
}

#[tokio::test]
async fn error_description_invalid_issuer_for_wrong_iss() {
    let mock = MockServer::start().await;
    let kp = test_key_pair();
    setup_mock_jwks(&mock, &kp).await;

    let cfg = make_cfg(&mock);
    let app = make_app(cfg).await;

    let now = now_secs();
    let claims = json!({
        "sub": "u",
        "iss": "https://wrong.example.com/realm",
        "exp": now + 3600i64,
    });
    let token = mint_jwt(&kp, claims);

    let req = axum::http::Request::builder()
        .uri("/protected")
        .header("authorization", format!("Bearer {}", token))
        .body(axum::body::Body::empty())
        .unwrap();

    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), 401);
    let www = resp
        .headers()
        .get("www-authenticate")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    assert_eq!(
        extract_error_description(www),
        Some("invalid_issuer"),
        "wrong issuer must produce error_description=invalid_issuer; got: {:?}",
        www
    );
}

#[tokio::test]
async fn error_description_invalid_audience_for_wrong_aud() {
    let mock = MockServer::start().await;
    let kp = test_key_pair();
    setup_mock_jwks(&mock, &kp).await;

    let mut cfg = make_cfg(&mock);
    cfg.audience = AudiencePolicy::Require("my-api".to_string());
    let issuer = cfg.issuer_url.clone();
    let app = make_app(cfg).await;

    let now = now_secs();
    let claims = json!({
        "sub": "u", "iss": issuer,
        "aud": "wrong-api",
        "exp": now + 3600i64,
    });
    let token = mint_jwt(&kp, claims);

    let req = axum::http::Request::builder()
        .uri("/protected")
        .header("authorization", format!("Bearer {}", token))
        .body(axum::body::Body::empty())
        .unwrap();

    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), 401);
    let www = resp
        .headers()
        .get("www-authenticate")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    assert_eq!(
        extract_error_description(www),
        Some("invalid_audience"),
        "wrong audience must produce error_description=invalid_audience; got: {:?}",
        www
    );
}

// ── A4: WWW-Authenticate space after comma ────────────────────────────────────

#[tokio::test]
async fn www_authenticate_has_space_after_comma_between_params() {
    let mock = MockServer::start().await;
    let kp = test_key_pair();
    setup_mock_jwks(&mock, &kp).await;

    let cfg = make_cfg(&mock);
    let issuer = cfg.issuer_url.clone();
    let app = make_app(cfg).await;

    // Send an expired token — triggers invalid_token + error_description
    let claims = json!({
        "sub": "u",
        "iss": issuer,
        "exp": 1000i64, // far in the past
        "iat": 900i64,
    });
    let token = mint_jwt(&kp, claims);

    let req = axum::http::Request::builder()
        .uri("/protected")
        .header("authorization", format!("Bearer {}", token))
        .body(axum::body::Body::empty())
        .unwrap();

    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), 401);

    let www = resp
        .headers()
        .get("www-authenticate")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");

    // RFC 6750 §3: auth-params separated by ", " (comma + space)
    assert!(
        www.contains(", error_description="),
        "WWW-Authenticate must have space after comma: got {:?}",
        www
    );
}

// ── #9: JWKS refetch single-flight (ADR 0070) ────────────────────────────────
//
// When many requests hit a cold (or stale) cache at once, the layer must
// coalesce their JWKS fetches into ONE outbound request — not one per inbound
// request. This bounds outbound fetch concurrency to 1 regardless of inbound
// load, the concurrency half of the forged-`kid` amplification defense.
//
// The JWKS endpoint responds slowly (set_delay) so the burst genuinely overlaps
// inside the fetch window. Without single-flight, all N requests fetch
// concurrently (N hits); with it, exactly one fetch occurs and the rest share it.

#[tokio::test]
async fn concurrent_cold_cache_requests_coalesce_into_single_jwks_fetch() {
    let server = MockServer::start().await;
    setup_mock_discovery(&server).await;
    let kp = test_key_pair();

    // Slow JWKS response so concurrent fetches overlap in the fetch window.
    Mock::given(method("GET"))
        .and(path("/jwks"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_delay(std::time::Duration::from_millis(300))
                .set_body_json(json!({ "keys": [jwk_for_key(&kp)] })),
        )
        .mount(&server)
        .await;

    let cfg = make_cfg(&server);
    let issuer = cfg.issuer_url.clone();
    let layer = oidc_validation_layer(cfg).expect("layer");

    async fn protected(claims: OidcClaims) -> impl IntoResponse {
        axum::Json(json!({ "sub": claims.sub }))
    }
    let inner = Router::new().route("/protected", get(protected));
    let app = Router::new().fallback_service(layer.layer(inner));

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let base_url = format!("http://127.0.0.1:{}", port);

    let now = now_secs();
    let token = mint_jwt(
        &kp,
        json!({ "sub": "u", "iss": issuer, "exp": now + 3600i64 }),
    );

    // Fire N concurrent requests against the cold cache.
    const N: usize = 10;
    let mut handles = Vec::with_capacity(N);
    for _ in 0..N {
        let url = format!("{}/protected", base_url);
        let bearer = format!("Bearer {}", token);
        handles.push(tokio::spawn(async move {
            reqwest::Client::new()
                .get(url)
                .header("authorization", bearer)
                .send()
                .await
                .unwrap()
                .status()
                .as_u16()
        }));
    }
    for h in handles {
        assert_eq!(
            h.await.unwrap(),
            200,
            "every concurrent request must succeed"
        );
    }

    let jwks_hits = server
        .received_requests()
        .await
        .unwrap()
        .iter()
        .filter(|r| r.url.path() == "/jwks")
        .count();
    assert_eq!(
        jwks_hits, 1,
        "single-flight: {N} concurrent cold-cache requests must coalesce into exactly 1 JWKS fetch, got {jwks_hits}"
    );
}

// During a real key rotation, a burst of requests carrying the freshly-rotated
// (legitimate, resolvable) kid must all succeed via ONE shared forced refetch —
// no thundering herd, and no spurious 401s while the single fetch is in flight.
// This is the ADR 0070 headline guarantee for the unknown-kid path.

#[tokio::test]
async fn concurrent_rotation_requests_share_one_refetch_no_spurious_401() {
    let server = MockServer::start().await;
    setup_mock_discovery(&server).await;
    let kp_old = test_key_pair();
    let kp_new = TestKeyPair {
        kid: "test-kid-2".to_string(),
        ..test_key_pair()
    };

    // Fetch 1 (priming): only the old key.
    Mock::given(method("GET"))
        .and(path("/jwks"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "keys": [jwk_for_key(&kp_old)]
        })))
        .up_to_n_times(1)
        .mount(&server)
        .await;

    // Fetch 2+ (post-rotation): both keys, served slowly so the burst overlaps.
    Mock::given(method("GET"))
        .and(path("/jwks"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_delay(std::time::Duration::from_millis(300))
                .set_body_json(json!({ "keys": [jwk_for_key(&kp_old), jwk_for_key(&kp_new)] })),
        )
        .mount(&server)
        .await;

    // Rate-limit disabled so the rotation refetch is allowed; single-flight is
    // then the only thing that can bound the burst to one fetch.
    let cfg = OidcValidationConfig {
        min_refetch_interval: std::time::Duration::from_secs(0),
        ..make_cfg(&server)
    };
    let issuer = cfg.issuer_url.clone();
    let layer = oidc_validation_layer(cfg).expect("layer");

    async fn protected(claims: OidcClaims) -> impl IntoResponse {
        axum::Json(json!({ "sub": claims.sub }))
    }
    let inner = Router::new().route("/protected", get(protected));
    let app = Router::new().fallback_service(layer.layer(inner));

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let base_url = format!("http://127.0.0.1:{}", port);
    let now = now_secs();

    // Prime the cache with kp_old (fetch 1).
    let t_old = mint_jwt(
        &kp_old,
        json!({ "sub": "u", "iss": issuer, "exp": now + 3600i64 }),
    );
    let r0 = reqwest::Client::new()
        .get(format!("{}/protected", base_url))
        .header("authorization", format!("Bearer {}", t_old))
        .send()
        .await
        .unwrap();
    assert_eq!(r0.status(), 200, "priming request must succeed");

    // Burst: N concurrent requests carrying the rotated kid (unknown to cache).
    const N: usize = 10;
    let t_new = mint_jwt(
        &kp_new,
        json!({ "sub": "u", "iss": issuer, "exp": now + 3600i64 }),
    );
    let mut handles = Vec::with_capacity(N);
    for _ in 0..N {
        let url = format!("{}/protected", base_url);
        let bearer = format!("Bearer {}", t_new);
        handles.push(tokio::spawn(async move {
            reqwest::Client::new()
                .get(url)
                .header("authorization", bearer)
                .send()
                .await
                .unwrap()
                .status()
                .as_u16()
        }));
    }
    for h in handles {
        assert_eq!(
            h.await.unwrap(),
            200,
            "every rotated-kid request must succeed — no spurious 401 during the shared refetch"
        );
    }

    let jwks_hits = server
        .received_requests()
        .await
        .unwrap()
        .iter()
        .filter(|r| r.url.path() == "/jwks")
        .count();
    assert_eq!(
        jwks_hits, 2,
        "expected exactly 2 JWKS fetches (1 prime + 1 coalesced rotation refetch), got {jwks_hits}"
    );
}

// ── Spec 018: OidcValidator tests ─────────────────────────────────────────────

use cli_framework_oidc::server::{OidcValidationError, OidcValidator, TokenRejection};

// Helper: build a validator pointing at the mock server.
fn make_validator(server: &MockServer) -> OidcValidator {
    OidcValidator::new(make_cfg(server)).expect("validator")
}

// ── validate() success path ───────────────────────────────────────────────────

#[tokio::test]
async fn validator_validate_success() {
    let server = MockServer::start().await;
    setup_mock_discovery(&server).await;
    let kp = test_key_pair();
    setup_mock_jwks(&server, &kp).await;

    let mut cfg = make_cfg(&server);
    cfg.audience = AudiencePolicy::Require("my-app".into());
    let validator = OidcValidator::new(cfg).expect("validator");

    let now = now_secs();
    let claims_val = json!({
        "sub": "user-abc",
        "iss": server.uri(),
        "aud": "my-app",
        "exp": now + 3600,
        "iat": now,
        "scope": "read write",
        "realm_access": { "roles": ["admin"] },
    });
    let token = mint_jwt(&kp, claims_val);

    let result = validator.validate(&token).await.expect("should succeed");
    assert_eq!(result.sub, "user-abc");
    assert!(result.scopes.contains(&"read".to_string()));
    assert!(result.scopes.contains(&"write".to_string()));
    assert!(result.roles.contains(&"admin".to_string()));
    assert_eq!(result.aud, vec!["my-app".to_string()]);
    // raw should contain the original claims
    assert_eq!(result.raw["sub"].as_str(), Some("user-abc"));
}

// ── validate() typed errors ───────────────────────────────────────────────────

#[tokio::test]
async fn validator_unsupported_algorithm() {
    let server = MockServer::start().await;
    setup_mock_discovery(&server).await;
    let kp = test_key_pair();
    setup_mock_jwks(&server, &kp).await;

    // Validator only accepts RS256 but token is ES256
    let mut cfg = make_cfg(&server);
    cfg.algorithms = vec![Algorithm::RS256];
    let validator = OidcValidator::new(cfg).expect("validator");

    let now = now_secs();
    let token = mint_jwt(
        &kp,
        json!({ "sub": "u", "iss": server.uri(), "exp": now + 3600i64 }),
    );

    let err = validator.validate(&token).await.unwrap_err();
    assert_eq!(
        err,
        OidcValidationError::InvalidToken(TokenRejection::UnsupportedAlgorithm),
        "wrong algorithm should yield UnsupportedAlgorithm"
    );
}

#[tokio::test]
async fn validator_unknown_key() {
    let server = MockServer::start().await;
    setup_mock_discovery(&server).await;
    let kp = test_key_pair();
    setup_mock_jwks(&server, &kp).await;

    let validator = make_validator(&server);

    let now = now_secs();
    let token = mint_jwt_with_kid(
        &kp,
        json!({ "sub": "u", "iss": server.uri(), "exp": now + 3600i64 }),
        "nonexistent-kid",
    );

    let err = validator.validate(&token).await.unwrap_err();
    assert_eq!(
        err,
        OidcValidationError::InvalidToken(TokenRejection::UnknownKey),
        "unknown kid should yield UnknownKey"
    );
}

#[tokio::test]
async fn validator_missing_sub_is_malformed() {
    let server = MockServer::start().await;
    setup_mock_discovery(&server).await;
    let kp = test_key_pair();
    setup_mock_jwks(&server, &kp).await;

    let validator = make_validator(&server);

    let now = now_secs();
    // No sub claim
    let token = mint_jwt(&kp, json!({ "iss": server.uri(), "exp": now + 3600i64 }));

    let err = validator.validate(&token).await.unwrap_err();
    assert_eq!(
        err,
        OidcValidationError::InvalidToken(TokenRejection::Malformed),
        "missing sub should yield Malformed"
    );
}

#[tokio::test]
async fn validator_expired_token() {
    let server = MockServer::start().await;
    setup_mock_discovery(&server).await;
    let kp = test_key_pair();
    setup_mock_jwks(&server, &kp).await;

    let mut cfg = make_cfg(&server);
    cfg.clock_skew = std::time::Duration::from_secs(0);
    let validator = OidcValidator::new(cfg).expect("validator");

    let now = now_secs();
    let token = mint_jwt(
        &kp,
        json!({ "sub": "u", "iss": server.uri(), "exp": now - 3600i64, "iat": now - 7200i64 }),
    );

    let err = validator.validate(&token).await.unwrap_err();
    assert_eq!(
        err,
        OidcValidationError::InvalidToken(TokenRejection::Expired),
        "expired token should yield Expired"
    );
}

#[tokio::test]
async fn validator_wrong_issuer() {
    let server = MockServer::start().await;
    setup_mock_discovery(&server).await;
    let kp = test_key_pair();
    setup_mock_jwks(&server, &kp).await;

    let validator = make_validator(&server);

    let now = now_secs();
    let token = mint_jwt(
        &kp,
        json!({ "sub": "u", "iss": "https://wrong.example.com", "exp": now + 3600i64 }),
    );

    let err = validator.validate(&token).await.unwrap_err();
    assert_eq!(
        err,
        OidcValidationError::InvalidToken(TokenRejection::InvalidIssuer),
        "wrong issuer should yield InvalidIssuer"
    );
}

#[tokio::test]
async fn validator_wrong_audience() {
    let server = MockServer::start().await;
    setup_mock_discovery(&server).await;
    let kp = test_key_pair();
    setup_mock_jwks(&server, &kp).await;

    let mut cfg = make_cfg(&server);
    cfg.audience = AudiencePolicy::Require("correct-api".into());
    let validator = OidcValidator::new(cfg).expect("validator");

    let now = now_secs();
    let token = mint_jwt(
        &kp,
        json!({ "sub": "u", "iss": server.uri(), "aud": "wrong-api", "exp": now + 3600i64 }),
    );

    let err = validator.validate(&token).await.unwrap_err();
    assert_eq!(
        err,
        OidcValidationError::InvalidToken(TokenRejection::InvalidAudience),
        "wrong audience should yield InvalidAudience"
    );
}

#[tokio::test]
async fn validator_bad_signature() {
    let server = MockServer::start().await;
    setup_mock_discovery(&server).await;
    let kp = test_key_pair();
    setup_mock_jwks(&server, &kp).await;

    let validator = make_validator(&server);

    // Sign with a different key but present kp.kid so the known-key path is taken.
    let other_kp = test_key_pair();
    let now = now_secs();
    let token = mint_jwt_with_kid(
        &other_kp,
        json!({ "sub": "u", "iss": server.uri(), "exp": now + 3600i64 }),
        &kp.kid,
    );

    let err = validator.validate(&token).await.unwrap_err();
    assert_eq!(
        err,
        OidcValidationError::InvalidToken(TokenRejection::InvalidSignature),
        "bad signature should yield InvalidSignature"
    );
}

#[tokio::test]
async fn validator_undecodable_token() {
    let server = MockServer::start().await;
    setup_mock_discovery(&server).await;
    let kp = test_key_pair();
    setup_mock_jwks(&server, &kp).await;

    let validator = make_validator(&server);

    // "not.a.jwt" has three segments but the header is not valid base64-JSON.
    // "notajwt" has no dots at all — decode_header fails immediately.
    let err = validator.validate("notajwt").await.unwrap_err();
    assert_eq!(
        err,
        OidcValidationError::InvalidToken(TokenRejection::Undecodable),
        "unparseable header should yield Undecodable"
    );
}

// ── validate() JWKS unavailable ───────────────────────────────────────────────

#[tokio::test]
async fn validator_jwks_unavailable() {
    let server = MockServer::start().await;

    // Discovery succeeds but JWKS returns 500
    Mock::given(method("GET"))
        .and(path("/.well-known/openid-configuration"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "issuer": server.uri(),
            "token_endpoint": format!("{}/token", server.uri()),
            "jwks_uri": format!("{}/jwks", server.uri()),
        })))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/jwks"))
        .respond_with(ResponseTemplate::new(500))
        .mount(&server)
        .await;

    let validator = make_validator(&server);
    let kp = test_key_pair();
    let now = now_secs();
    let token = mint_jwt(
        &kp,
        json!({ "sub": "u", "iss": server.uri(), "exp": now + 3600i64 }),
    );

    let err = validator.validate(&token).await.unwrap_err();
    assert_eq!(
        err,
        OidcValidationError::JwksUnavailable,
        "failed JWKS fetch with empty cache should yield JwksUnavailable"
    );
}

// ── validate_authorization() header parsing ───────────────────────────────────

#[tokio::test]
async fn validate_authorization_none_is_missing_token() {
    let server = MockServer::start().await;
    setup_mock_discovery(&server).await;
    let kp = test_key_pair();
    setup_mock_jwks(&server, &kp).await;

    let validator = make_validator(&server);
    let err = validator.validate_authorization(None).await.unwrap_err();
    assert_eq!(err, OidcValidationError::MissingToken);
}

#[tokio::test]
async fn validate_authorization_non_bearer_is_malformed() {
    let server = MockServer::start().await;
    setup_mock_discovery(&server).await;
    let kp = test_key_pair();
    setup_mock_jwks(&server, &kp).await;

    let validator = make_validator(&server);
    let err = validator
        .validate_authorization(Some("Basic dXNlcjpwYXNz"))
        .await
        .unwrap_err();
    assert_eq!(err, OidcValidationError::MalformedAuthorization);
}

#[tokio::test]
async fn validate_authorization_mixed_case_bearer_accepted() {
    let server = MockServer::start().await;
    setup_mock_discovery(&server).await;
    let kp = test_key_pair();
    setup_mock_jwks(&server, &kp).await;

    let validator = make_validator(&server);

    let now = now_secs();
    let token = mint_jwt(
        &kp,
        json!({ "sub": "u", "iss": server.uri(), "exp": now + 3600i64 }),
    );

    // "bEaReR " prefix (case-insensitive)
    let result = validator
        .validate_authorization(Some(&format!("bEaReR {token}")))
        .await;
    assert!(
        result.is_ok(),
        "mixed-case Bearer should be accepted; got: {:?}",
        result
    );
}

#[tokio::test]
async fn validate_authorization_normal_bearer_accepted() {
    let server = MockServer::start().await;
    setup_mock_discovery(&server).await;
    let kp = test_key_pair();
    setup_mock_jwks(&server, &kp).await;

    let validator = make_validator(&server);

    let now = now_secs();
    let token = mint_jwt(
        &kp,
        json!({ "sub": "u", "iss": server.uri(), "exp": now + 3600i64 }),
    );

    let result = validator
        .validate_authorization(Some(&format!("Bearer {token}")))
        .await;
    assert!(result.is_ok(), "standard Bearer should be accepted");
}

// ── Layer non-UTF-8 header parity ─────────────────────────────────────────────
//
// A present-but-non-UTF-8 Authorization header must produce
// 401 + Bearer error="invalid_request" (MalformedAuthorization path).

#[tokio::test]
async fn layer_non_utf8_header_returns_401_invalid_request() {
    let server = MockServer::start().await;
    setup_mock_discovery(&server).await;
    let kp = test_key_pair();
    setup_mock_jwks(&server, &kp).await;

    let app = make_app(make_cfg(&server)).await;

    // Build a request with a non-UTF-8 Authorization header value.
    let non_utf8_bytes = vec![0xFF, 0xFE];
    let hv = axum::http::HeaderValue::from_bytes(&non_utf8_bytes).unwrap();
    let mut req = axum::http::Request::builder()
        .uri("/protected")
        .body(axum::body::Body::empty())
        .unwrap();
    req.headers_mut().insert("authorization", hv);

    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), axum::http::StatusCode::UNAUTHORIZED);
    let www = resp
        .headers()
        .get("www-authenticate")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    assert!(
        www.contains("error=\"invalid_request\""),
        "non-UTF-8 header must produce invalid_request; got: {www:?}"
    );
}

// ── error_to_response wire parity ─────────────────────────────────────────────
//
// Validate through the layer that:
// - Undecodable -> bare error="invalid_token" with NO error_description
// - Malformed   -> error_description="malformed_token"

#[tokio::test]
async fn error_to_response_undecodable_has_no_error_description() {
    let server = MockServer::start().await;
    setup_mock_discovery(&server).await;
    let kp = test_key_pair();
    setup_mock_jwks(&server, &kp).await;

    let app = make_app(make_cfg(&server)).await;

    // "not.a.jwt" - decode_header will fail -> Undecodable
    let req = axum::http::Request::builder()
        .uri("/protected")
        .header("authorization", "Bearer notajwt")
        .body(axum::body::Body::empty())
        .unwrap();

    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), axum::http::StatusCode::UNAUTHORIZED);
    let www = resp
        .headers()
        .get("www-authenticate")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    assert!(
        www.contains("error=\"invalid_token\""),
        "Undecodable must emit error=invalid_token; got: {www:?}"
    );
    assert!(
        !www.contains("error_description"),
        "Undecodable must NOT emit error_description; got: {www:?}"
    );
}

#[tokio::test]
async fn error_to_response_malformed_emits_malformed_token() {
    let server = MockServer::start().await;
    setup_mock_discovery(&server).await;
    let kp = test_key_pair();
    setup_mock_jwks(&server, &kp).await;

    let app = make_app(make_cfg(&server)).await;

    let now = now_secs();
    // Token without sub -> Malformed
    let token = mint_jwt(&kp, json!({ "iss": server.uri(), "exp": now + 3600i64 }));

    let req = axum::http::Request::builder()
        .uri("/protected")
        .header("authorization", format!("Bearer {token}"))
        .body(axum::body::Body::empty())
        .unwrap();

    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), axum::http::StatusCode::UNAUTHORIZED);
    let www = resp
        .headers()
        .get("www-authenticate")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    assert_eq!(
        extract_error_description(www),
        Some("malformed_token"),
        "missing sub must emit error_description=malformed_token; got: {www:?}"
    );
}

// ── Shared state across clones: single JWKS fetch ─────────────────────────────
//
// Two clones of one OidcValidator issuing concurrent validate() calls for the
// same kid must trigger AT MOST ONE JWKS fetch (ADR-0070 single-flight preserved).

#[tokio::test]
async fn validator_clones_share_jwks_state_single_fetch() {
    let server = MockServer::start().await;
    setup_mock_discovery(&server).await;
    let kp = test_key_pair();

    // Slow JWKS so concurrent fetches would overlap if not gated.
    Mock::given(method("GET"))
        .and(path("/jwks"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_delay(std::time::Duration::from_millis(200))
                .set_body_json(json!({ "keys": [jwk_for_key(&kp)] })),
        )
        .mount(&server)
        .await;

    let validator = make_validator(&server);
    let validator2 = validator.clone();

    let issuer = server.uri();
    let now = now_secs();
    let token = mint_jwt(
        &kp,
        json!({ "sub": "u", "iss": issuer, "exp": now + 3600i64 }),
    );
    let token2 = token.clone();

    // Fire two concurrent validate() calls from two clones simultaneously.
    let h1 = tokio::spawn(async move { validator.validate(&token).await });
    let h2 = tokio::spawn(async move { validator2.validate(&token2).await });

    let r1 = h1.await.unwrap();
    let r2 = h2.await.unwrap();
    assert!(r1.is_ok(), "clone 1 validate should succeed: {:?}", r1);
    assert!(r2.is_ok(), "clone 2 validate should succeed: {:?}", r2);

    let jwks_hits = server
        .received_requests()
        .await
        .unwrap()
        .iter()
        .filter(|r| r.url.path() == "/jwks")
        .count();
    assert!(
        jwks_hits <= 1,
        "shared clones must coalesce into at most 1 JWKS fetch, got {jwks_hits}"
    );
}
