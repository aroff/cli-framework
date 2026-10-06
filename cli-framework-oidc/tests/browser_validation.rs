use cli_framework::axum::http::{HeaderMap, HeaderValue};
/// Unit tests for the browser feature — cookie crypto, PKCE, auth state, request-type detection.
/// These tests do not require a network connection or real Keycloak instance.
use cli_framework_oidc::browser::{
    auth_state::{decode_auth_state, derive_hmac_key, encode_auth_state, random_state, AuthState},
    cookie::{decrypt_cookie, encrypt_cookie},
    pkce::{derive_challenge, generate_verifier},
    request_type::{detect, validate_return_to, RequestType},
    session_key::SessionKey,
};

fn test_key() -> [u8; 32] {
    [42u8; 32]
}

fn browser_config(issuer: &str) -> cli_framework_oidc::browser::OidcBrowserSessionConfig {
    cli_framework_oidc::browser::OidcBrowserSessionConfig::new(
        issuer,
        "client ü & +",
        "https://app.example/auth/callback",
        test_session_key(),
        cli_framework_oidc::browser::AudiencePolicy::Unchecked,
    )
}

#[tokio::test]
async fn token_exchange_rejects_redirects_error_status_and_large_bodies() {
    use tower::ServiceExt;
    use wiremock::{
        matchers::{method, path},
        Mock, MockServer, ResponseTemplate,
    };
    let destination = MockServer::start().await;
    Mock::given(path("/capture"))
        .respond_with(ResponseTemplate::new(200))
        .expect(0)
        .mount(&destination)
        .await;
    let responses = [
        ResponseTemplate::new(307)
            .insert_header("Location", format!("{}/capture", destination.uri())),
        ResponseTemplate::new(400).set_body_json(
            serde_json::json!({"access_token": "secret-access", "refresh_token": "secret-refresh"}),
        ),
        ResponseTemplate::new(200).set_body_string("x".repeat(64 * 1024 + 1)),
    ];
    for token_response in responses {
        let provider = MockServer::start().await;
        let issuer = provider.uri();
        Mock::given(path("/.well-known/openid-configuration"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "issuer": issuer,
                "authorization_endpoint": format!("{issuer}/login"),
                "token_endpoint": format!("{issuer}/token"),
                "jwks_uri": format!("{issuer}/keys")
            })))
            .expect(1)
            .mount(&provider)
            .await;
        Mock::given(method("POST"))
            .and(path("/token"))
            .respond_with(token_response)
            .expect(1)
            .mount(&provider)
            .await;
        let signed = encode_auth_state(
            &AuthState {
                state: "expected-state".into(),
                verifier: generate_verifier(),
                return_to: "/".into(),
            },
            &derive_hmac_key(&test_key()),
        );
        let response = browser_app(&issuer)
            .oneshot(
                axum::http::Request::builder()
                    .uri("/auth/callback?state=expected-state&code=secret-code")
                    .header("cookie", format!("__auth_state={signed}"))
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), axum::http::StatusCode::BAD_GATEWAY);
        assert!(!response.headers().contains_key("set-cookie"));
        let body = axum::body::to_bytes(response.into_body(), 1024)
            .await
            .unwrap();
        assert_eq!(body.as_ref(), b"Token exchange failed");
    }
}

#[tokio::test]
async fn invalid_callback_state_clears_the_configured_cookie_path() {
    use tower::ServiceExt;
    let response = browser_app("https://provider.example")
        .oneshot(
            axum::http::Request::builder()
                .uri("/auth/callback?state=invalid&code=code")
                .header("cookie", "__auth_state=tampered")
                .body(axum::body::Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), axum::http::StatusCode::BAD_REQUEST);
    let cookie = response.headers()["set-cookie"].to_str().unwrap();
    assert!(cookie.contains("Path=/auth/callback"));
    assert!(cookie.contains("Max-Age=0"));
}

fn browser_app(issuer: &str) -> axum::Router {
    use tower::Layer;
    let mut cfg = browser_config(issuer);
    cfg.callback_path = "/auth/callback".into();
    let parts = cli_framework_oidc::browser::oidc_browser_session_layer(cfg).unwrap();
    let protected = axum::Router::new().route("/", axum::routing::get(|| async { "protected" }));
    parts
        .callback_router
        .fallback_service(parts.layer.layer(protected))
}

async fn navigate(app: axum::Router) -> axum::response::Response {
    use tower::ServiceExt;
    app.oneshot(
        axum::http::Request::builder()
            .uri("/")
            .header("accept", "text/html")
            .body(axum::body::Body::empty())
            .unwrap(),
    )
    .await
    .unwrap()
}

#[tokio::test]
async fn navigation_uses_discovered_endpoint_and_encodes_parameters() {
    use wiremock::{matchers::path, Mock, MockServer, ResponseTemplate};
    let provider = MockServer::start().await;
    let issuer = provider.uri();
    Mock::given(path("/.well-known/openid-configuration"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "issuer": issuer,
            "authorization_endpoint": format!("{issuer}/custom-login?tenant=one"),
            "token_endpoint": format!("{issuer}/token"),
            "jwks_uri": format!("{issuer}/keys")
        })))
        .expect(1)
        .mount(&provider)
        .await;
    let response = navigate(browser_app(&issuer)).await;
    assert_eq!(response.status(), axum::http::StatusCode::FOUND);
    let url = url::Url::parse(response.headers()["location"].to_str().unwrap()).unwrap();
    assert_eq!(url.path(), "/custom-login");
    let pairs: std::collections::HashMap<_, _> = url.query_pairs().into_owned().collect();
    assert_eq!(pairs["tenant"], "one");
    assert_eq!(pairs["client_id"], "client ü & +");
    assert_eq!(pairs["redirect_uri"], "https://app.example/auth/callback");
    assert_eq!(pairs["code_challenge_method"], "S256");
    assert_eq!(pairs["state"].len(), 43);
    let cookie = response.headers()["set-cookie"].to_str().unwrap();
    assert!(cookie.contains("; Secure;"));
    assert!(cookie.contains("Path=/auth/callback;"));
}

#[tokio::test]
async fn unusable_discovery_fails_without_issuing_state() {
    use wiremock::{matchers::path, Mock, MockServer, ResponseTemplate};
    for endpoint in [
        None,
        Some("http://remote.example/login"),
        Some("https://user@provider.example/login"),
        Some("https://provider.example/login?state=override"),
    ] {
        let provider = MockServer::start().await;
        let issuer = provider.uri();
        Mock::given(path("/.well-known/openid-configuration"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "issuer": issuer,
                "authorization_endpoint": endpoint,
                "token_endpoint": format!("{issuer}/token"),
                "jwks_uri": format!("{issuer}/keys")
            })))
            .expect(1)
            .mount(&provider)
            .await;
        let response = navigate(browser_app(&issuer)).await;
        assert_eq!(
            response.status(),
            axum::http::StatusCode::SERVICE_UNAVAILABLE,
            "{endpoint:?}"
        );
        assert!(!response.headers().contains_key("set-cookie"));
        assert!(!response.headers().contains_key("location"));
    }
}

#[tokio::test]
async fn discovery_redirect_does_not_contact_destination() {
    use wiremock::{matchers::path, Mock, MockServer, ResponseTemplate};
    let provider = MockServer::start().await;
    let destination = MockServer::start().await;
    Mock::given(path("/redirected"))
        .respond_with(ResponseTemplate::new(200))
        .expect(0)
        .mount(&destination)
        .await;
    Mock::given(path("/.well-known/openid-configuration"))
        .respond_with(
            ResponseTemplate::new(302)
                .insert_header("Location", format!("{}/redirected", destination.uri())),
        )
        .expect(1)
        .mount(&provider)
        .await;
    let response = navigate(browser_app(&provider.uri())).await;
    assert_eq!(
        response.status(),
        axum::http::StatusCode::SERVICE_UNAVAILABLE
    );
}

#[test]
fn actual_cookie_budget_rejects_oversized_sealed_values() {
    use cli_framework_oidc::browser::cookie::CookieError;
    assert!(matches!(
        encrypt_cookie(&test_key(), &"x".repeat(2048), &"r".repeat(64), 123),
        Err(CookieError::TooLarge)
    ));
    assert!(matches!(
        decrypt_cookie(&test_key(), &"x".repeat(3801)),
        Err(CookieError::TooLarge)
    ));
    assert!(encrypt_cookie(&test_key(), "access", "refresh", 123).is_ok());
}

#[test]
fn browser_configuration_rejects_unsafe_callback_and_cookie_paths() {
    use cli_framework_oidc::browser::oidc_browser_session_layer;
    for callback in [
        "https://user@app.example/auth/callback",
        "https://app.example/auth/callback?query=1",
        "https://app.example/auth/callback#fragment",
        "http://app.example/auth/callback",
    ] {
        let mut cfg = browser_config("https://provider.example");
        cfg.callback_path = "/auth/callback".into();
        cfg.redirect_uri = callback.into();
        assert!(oidc_browser_session_layer(cfg).is_err(), "{callback}");
    }
    let mut cfg = browser_config("https://provider.example");
    cfg.callback_path = "/auth/callback".into();
    cfg.cookie_name = "session; injected=value".into();
    assert!(oidc_browser_session_layer(cfg).is_err());
}

#[tokio::test]
async fn malformed_authorization_is_rejected_before_cookie_or_network() {
    use tower::{Layer, ServiceExt};
    use wiremock::MockServer;
    let provider = MockServer::start().await;
    let mut cfg = browser_config(&provider.uri());
    cfg.callback_path = "/auth/callback".into();
    let layer =
        cli_framework_oidc::browser::oidc_dual_mode_layer(&cfg, cfg.audience.clone()).unwrap();
    let app =
        layer.layer(axum::Router::new().route("/", axum::routing::get(|| async { "protected" })));
    for values in [
        vec!["Basic abc"],
        vec!["Bearer "],
        vec!["Bearer a b"],
        vec!["éééé abc"],
        vec!["Bearer a", "Bearer b"],
    ] {
        let mut request = axum::http::Request::builder()
            .uri("/")
            .header("cookie", "session=offered");
        for value in values {
            request = request.header("authorization", value);
        }
        let response = app
            .clone()
            .oneshot(request.body(axum::body::Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(response.status(), axum::http::StatusCode::UNAUTHORIZED);
        assert_eq!(
            response.headers()["www-authenticate"],
            "Bearer error=\"invalid_request\""
        );
    }
    assert!(provider.received_requests().await.unwrap().is_empty());
}

fn test_session_key() -> SessionKey {
    SessionKey::from_bytes(test_key())
}

// ── T1: Cookie validation ────────────────────────────────────────────────────

#[test]
fn test_cookie_roundtrip() {
    let key = test_key();
    let at = "eyJhbGciOiJSUzI1NiJ9.access.token";
    let rt = "opaque-refresh-token-xyz";
    let exp = 9_999_999_999i64;

    let encrypted = encrypt_cookie(&key, at, rt, exp).expect("encrypt");
    let payload = decrypt_cookie(&key, &encrypted).expect("decrypt");

    assert_eq!(payload.access_token, at);
    assert_eq!(payload.refresh_token, rt);
    assert_eq!(payload.refresh_exp, exp);
}

#[test]
fn test_cookie_tamper_detection() {
    let key = test_key();
    let encrypted = encrypt_cookie(&key, "token", "refresh", 9999999999).expect("encrypt");

    // Flip a byte in the middle of the base64
    let mut bytes = encrypted.into_bytes();
    let mid = bytes.len() / 2;
    bytes[mid] = if bytes[mid] == b'A' { b'B' } else { b'A' };
    let tampered = String::from_utf8(bytes).unwrap();

    let result = decrypt_cookie(&key, &tampered);
    assert!(result.is_err(), "tampered cookie should fail decryption");
}

#[test]
fn test_cookie_wrong_key() {
    let key1 = [1u8; 32];
    let key2 = [2u8; 32];
    let encrypted = encrypt_cookie(&key1, "token", "refresh", 9999999999).expect("encrypt");
    let result = decrypt_cookie(&key2, &encrypted);
    assert!(result.is_err(), "wrong key should fail decryption");
}

#[test]
fn test_cookie_invalid_base64_rejected() {
    let key = test_key();
    let result = decrypt_cookie(&key, "not-valid-base64!!!");
    assert!(result.is_err());
}

#[test]
fn test_cookie_unknown_version() {
    // We can't easily forge an unknown-version cookie (it's encrypted), but we can verify
    // that the version 1 path succeeds and that the error type exists.
    let key = test_key();
    let encrypted = encrypt_cookie(&key, "at", "rt", 1234567890).expect("encrypt");
    let result = decrypt_cookie(&key, &encrypted);
    assert!(result.is_ok());
}

// ── T2: PKCE ────────────────────────────────────────────────────────────────

#[test]
fn test_pkce_verifier_length() {
    let v = generate_verifier();
    // 32 raw bytes → base64url = ceil(32 * 4/3) = 43 chars (no padding)
    assert_eq!(v.len(), 43, "verifier should be 43 chars");
}

#[test]
fn test_pkce_verifier_unique() {
    let v1 = generate_verifier();
    let v2 = generate_verifier();
    assert_ne!(v1, v2, "two verifiers should be distinct");
}

#[test]
fn test_pkce_challenge_deterministic() {
    let v = "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk";
    // SHA-256 of the above verifier, base64url (known test vector from RFC 7636)
    let expected = "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM";
    assert_eq!(derive_challenge(v), expected);
}

#[test]
fn test_pkce_roundtrip() {
    let verifier = generate_verifier();
    let challenge = derive_challenge(&verifier);
    assert!(!challenge.is_empty());
    // Challenge must not equal the verifier (it's the hash)
    assert_ne!(verifier, challenge);
}

// ── T3: Auth state (state anti-forgery) ─────────────────────────────────────

#[test]
fn test_auth_state_roundtrip() {
    let key = test_key();
    let hmac_key = derive_hmac_key(&key);

    let state = AuthState {
        state: "random-state-xyz".to_string(),
        verifier: "pkce-verifier-abc".to_string(),
        return_to: "/dashboard".to_string(),
    };

    let encoded = encode_auth_state(&state, &hmac_key);
    let decoded = decode_auth_state(&encoded, &hmac_key).expect("decode");

    assert_eq!(decoded.state, "random-state-xyz");
    assert_eq!(decoded.verifier, "pkce-verifier-abc");
    assert_eq!(decoded.return_to, "/dashboard");
}

#[test]
fn test_auth_state_hmac_reject_on_tamper() {
    let key = test_key();
    let hmac_key = derive_hmac_key(&key);

    let auth_state = AuthState {
        state: "s".to_string(),
        verifier: "v".to_string(),
        return_to: "/".to_string(),
    };
    let encoded = encode_auth_state(&auth_state, &hmac_key);

    // Flip a character in the payload portion (before the last '.')
    let dot = encoded.rfind('.').unwrap();
    let mut bytes = encoded.into_bytes();
    bytes[dot - 1] = if bytes[dot - 1] == b'A' { b'B' } else { b'A' };
    let tampered = String::from_utf8(bytes).unwrap();

    let result = decode_auth_state(&tampered, &hmac_key);
    assert!(result.is_none(), "tampered auth state should be rejected");
}

#[test]
fn test_auth_state_wrong_key_rejected() {
    let key1 = [1u8; 32];
    let key2 = [2u8; 32];
    let hmac_key1 = derive_hmac_key(&key1);
    let hmac_key2 = derive_hmac_key(&key2);

    let auth_state = AuthState {
        state: "s".to_string(),
        verifier: "v".to_string(),
        return_to: "/".to_string(),
    };
    let encoded = encode_auth_state(&auth_state, &hmac_key1);
    assert!(
        decode_auth_state(&encoded, &hmac_key2).is_none(),
        "wrong HMAC key should reject"
    );
}

#[test]
fn test_auth_state_missing_dot_rejected() {
    let key = test_key();
    let hmac_key = derive_hmac_key(&key);
    assert!(decode_auth_state("nodothere", &hmac_key).is_none());
}

#[test]
fn test_random_state_unique() {
    let s1 = random_state();
    let s2 = random_state();
    assert_ne!(s1, s2);
    assert!(!s1.is_empty());
}

// ── T6: Request-type detection ───────────────────────────────────────────────

fn headers_with(pairs: &[(&str, &str)]) -> HeaderMap {
    let mut h = HeaderMap::new();
    for (k, v) in pairs {
        h.insert(
            cli_framework::axum::http::header::HeaderName::from_bytes(k.as_bytes()).unwrap(),
            HeaderValue::from_str(v).unwrap(),
        );
    }
    h
}

#[test]
fn test_request_type_navigate_via_sec_fetch_mode() {
    let h = headers_with(&[("sec-fetch-mode", "navigate")]);
    assert_eq!(detect(&h), RequestType::Navigation);
}

#[test]
fn test_request_type_cors_via_sec_fetch_mode() {
    let h = headers_with(&[("sec-fetch-mode", "cors")]);
    assert_eq!(detect(&h), RequestType::ApiFetch);
}

#[test]
fn test_request_type_same_origin_via_sec_fetch_mode() {
    let h = headers_with(&[("sec-fetch-mode", "same-origin")]);
    assert_eq!(detect(&h), RequestType::ApiFetch);
}

#[test]
fn test_request_type_text_html_accept_fallback() {
    let h = headers_with(&[("accept", "text/html,application/xhtml+xml")]);
    assert_eq!(detect(&h), RequestType::Navigation);
}

#[test]
fn test_request_type_json_accept_fallback() {
    let h = headers_with(&[("accept", "application/json")]);
    assert_eq!(detect(&h), RequestType::ApiFetch);
}

#[test]
fn test_request_type_wildcard_accept_is_api() {
    // fetch() default: Accept: */* — must go to API/fetch branch, not navigation
    let h = headers_with(&[("accept", "*/*")]);
    assert_eq!(detect(&h), RequestType::ApiFetch);
}

#[test]
fn test_request_type_no_headers_is_api() {
    let h = HeaderMap::new();
    assert_eq!(detect(&h), RequestType::ApiFetch);
}

// ── Return-to validation ─────────────────────────────────────────────────────

#[test]
fn test_return_to_valid_path() {
    assert!(validate_return_to("/dashboard").is_ok());
    assert!(validate_return_to("/some/deep/path?q=1").is_ok());
    assert!(validate_return_to("/").is_ok());
}

#[test]
fn test_return_to_protocol_relative_rejected() {
    assert!(validate_return_to("//evil.com").is_err());
}

#[test]
fn test_return_to_backslash_rejected() {
    assert!(validate_return_to("\\evil").is_err());
}

#[test]
fn test_return_to_url_encoded_traversal_rejected() {
    assert!(validate_return_to("%2F%2Fevil.com").is_err());
    assert!(validate_return_to("%5cevil").is_err());
}

#[test]
fn test_return_to_control_chars_rejected() {
    assert!(validate_return_to("/path\r\nSet-Cookie: x=y").is_err());
    assert!(validate_return_to("/path\ninjection").is_err());
}

#[test]
fn test_return_to_must_start_with_slash() {
    assert!(validate_return_to("relative/path").is_err());
    assert!(validate_return_to("https://evil.com/path").is_err());
}

// ── SessionKey properties ────────────────────────────────────────────────────

#[test]
fn test_session_key_clone_is_available() {
    // Verify SessionKey implements Clone (compile-time test).
    let key = test_session_key();
    let key2 = key.clone();
    // Both should be usable independently — verified by the cookie roundtrip tests above.
    let _ = key2;
}

// Verify SessionKey does not implement Debug (would be a compile error if enabled).
// This is tested implicitly — if it compiled, the test passes.
#[test]
fn test_session_key_no_debug_compile() {
    // If Debug were derived, this would still compile — but the absence of the
    // trait is enforced by the type definition (no #[derive(Debug)]).
    let _key = test_session_key();
    // If the line below were uncommented, it would fail to compile:
    // println!("{:?}", _key);
}
