//! Real router, discovery, code exchange and signed-token login contracts.
use axum::{
    body::Body,
    http::{Request, StatusCode},
    response::Response,
    Router,
};
use cli_framework_oidc::{
    browser::{
        Algorithm, AudiencePolicy, OidcBrowserSession, OidcBrowserSessionConfig, SessionKey,
    },
    test_support::{jwk_for_key, mint_jwt, now_secs, test_key_pair, TestKeyPair},
};
use serde_json::{json, Value};
use tower::{Layer, ServiceExt};
use wiremock::{
    matchers::{body_string_contains, method, path},
    Mock, MockServer, ResponseTemplate,
};

struct LoginFixture {
    provider: MockServer,
    key: TestKeyPair,
    app: Router,
    session: OidcBrowserSession,
    writes: std::sync::Arc<std::sync::atomic::AtomicUsize>,
}

async fn wait_for_refresh(fixture: &LoginFixture) {
    tokio::time::timeout(std::time::Duration::from_secs(2), async {
        loop {
            if fixture
                .provider
                .received_requests()
                .await
                .unwrap()
                .iter()
                .any(|request| {
                    request.url.path() == "/token"
                        && String::from_utf8_lossy(&request.body)
                            .contains("grant_type=refresh_token")
                })
            {
                return;
            }
            tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn cancelled_refresh_is_not_replayed_and_late_refresh_cannot_undo_logout() {
    for logout in [false, true] {
        let fixture = LoginFixture::new().await;
        let mut initial = fixture.access_claims();
        initial["exp"] = json!(now_secs() + 30);
        let cookie = fixture.session_cookie(initial).await;
        let id = cookie.strip_prefix("session=").unwrap().to_string();
        Mock::given(method("POST")).and(path("/token"))
            .and(body_string_contains("grant_type=refresh_token"))
            .respond_with(ResponseTemplate::new(200).set_delay(std::time::Duration::from_millis(300)).set_body_json(json!({"access_token": mint_jwt(&fixture.key, fixture.access_claims()), "token_type": "Bearer", "refresh_token": "rotated"})))
            .expect(1).mount(&fixture.provider).await;
        let operation = tokio::spawn({
            let session = fixture.session.clone();
            let id = id.clone();
            async move { session.authenticate_cookie(&id).await }
        });
        wait_for_refresh(&fixture).await;
        if logout {
            let response = fixture
                .app
                .clone()
                .oneshot(
                    Request::builder()
                        .method("POST")
                        .uri("/logout")
                        .header("origin", "https://app.example")
                        .header("cookie", &cookie)
                        .body(Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::FOUND);
            assert!(operation.await.unwrap().is_err());
            tokio::time::sleep(std::time::Duration::from_millis(350)).await;
            assert!(fixture.session.authenticate_cookie(&id).await.is_err());
        } else {
            operation.abort();
            assert!(matches!(operation.await, Err(error) if error.is_cancelled()));
            let access = fixture.session.authenticate_cookie(&id).await.unwrap();
            assert!(access.is_live());
            assert!(access.claims().exp < now_secs() + 60);
        }
    }
}

#[tokio::test]
async fn refreshed_identity_must_preserve_original_authentication() {
    for failure in [
        None,
        Some("nonce"),
        Some("subject"),
        Some("audience"),
        Some("auth-time"),
        Some("azp"),
        Some("signature"),
        Some("issue-time"),
        Some("malformed-token"),
    ] {
        let fixture = LoginFixture::new().await;
        let (state, nonce, login_cookie) = fixture.login().await;
        let mut identity = fixture.identity_claims(&nonce);
        identity["auth_time"] = json!(now_secs() - 20);
        let mut access = fixture.access_claims();
        access["exp"] = json!(now_secs() + 30);
        let initial_exp = access["exp"].as_i64().unwrap();
        fixture.tokens(Some(identity.clone()), access).await;
        let response = fixture.callback(&state, &login_cookie).await;
        assert_eq!(response.status(), StatusCode::FOUND);
        let cookie = response
            .headers()
            .get_all("set-cookie")
            .iter()
            .map(|value| value.to_str().unwrap())
            .find(|value| value.starts_with("session="))
            .unwrap()
            .split(';')
            .next()
            .unwrap();
        let id = cookie.strip_prefix("session=").unwrap();
        identity.as_object_mut().unwrap().remove("nonce");
        match failure {
            Some("nonce") => identity["nonce"] = json!("other"),
            Some("subject") => identity["sub"] = json!("other"),
            Some("audience") => {
                identity["aud"] = json!(["web-client", "other"]);
                identity["azp"] = json!("web-client");
            }
            Some("auth-time") => identity["auth_time"] = json!(now_secs()),
            Some("azp") => identity["azp"] = json!("web-client"),
            Some("issue-time") => identity["iat"] = json!(now_secs() - 30),
            _ => (),
        }
        let other_key = test_key_pair();
        let key = if failure == Some("signature") {
            &other_key
        } else {
            &fixture.key
        };
        let mut body = json!({"access_token": mint_jwt(&fixture.key, fixture.access_claims()), "token_type": "Bearer", "refresh_token": "rotated", "id_token": mint_jwt(key, identity)});
        if failure == Some("malformed-token") {
            body["refresh_token"] = json!({"malformed": true});
        }
        fixture.refresh_response(body).await;
        let access = fixture.session.authenticate_cookie(id).await.unwrap();
        if failure.is_some() {
            assert_eq!(access.claims().exp, initial_exp, "{failure:?}");
            assert_eq!(
                fixture
                    .session
                    .authenticate_cookie(id)
                    .await
                    .unwrap()
                    .claims()
                    .exp,
                initial_exp
            );
        } else {
            assert!(access.claims().exp > initial_exp);
        }
    }
}

#[tokio::test]
async fn large_provider_jwt_stays_on_server_while_cookie_remains_small() {
    let fixture = LoginFixture::new().await;
    let mut claims = fixture.access_claims();
    claims["large_profile"] = json!("x".repeat(6000));
    let cookie = fixture.session_cookie(claims).await;
    assert_eq!(cookie.len(), "session=".len() + 46);
    assert!(fixture
        .session
        .authenticate_cookie(cookie.strip_prefix("session=").unwrap())
        .await
        .unwrap()
        .is_live());
}

#[tokio::test]
async fn copied_cookie_and_stream_access_are_revoked_by_logout() {
    let fixture = LoginFixture::new().await;
    let cookie = fixture.session_cookie(fixture.access_claims()).await;
    let id = cookie.strip_prefix("session=").unwrap();
    assert!(id.starts_with("s2."));
    assert_eq!(id.len(), 46);
    let access = fixture.session.authenticate_cookie(id).await.unwrap();
    let waiter = tokio::spawn({
        let access = access.clone();
        async move { access.invalidated().await }
    });
    let response = fixture
        .app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/logout")
                .header("origin", "https://app.example")
                .header("cookie", &cookie)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::FOUND);
    let redirect = url::Url::parse(response.headers()["location"].to_str().unwrap()).unwrap();
    let params: std::collections::HashMap<_, _> = redirect.query_pairs().into_owned().collect();
    assert_eq!(params["tenant"], "one");
    assert_eq!(params["post_logout_redirect_uri"], "https://app.example/");
    tokio::time::timeout(std::time::Duration::from_secs(1), waiter)
        .await
        .unwrap()
        .unwrap();
    assert!(!access.is_live());
    assert!(fixture.session.authenticate_cookie(id).await.is_err());
    let replay = fixture
        .app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/identity")
                .header("cookie", cookie)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(replay.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn original_session_lifetime_invalidates_cookie_and_stream_access() {
    let fixture = LoginFixture::with_ttl(std::time::Duration::from_millis(200)).await;
    let cookie = fixture.session_cookie(fixture.access_claims()).await;
    let id = cookie.strip_prefix("session=").unwrap();
    let access = fixture.session.authenticate_cookie(id).await.unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(2), access.invalidated())
        .await
        .unwrap();
    assert!(!access.is_live());
    assert!(fixture.session.authenticate_cookie(id).await.is_err());
    let response = fixture
        .app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/identity")
                .header("cookie", cookie)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn concurrent_refresh_uses_one_provider_operation() {
    for lifetime in [30, 300] {
        let fixture = LoginFixture::new().await;
        let mut initial = fixture.access_claims();
        initial["exp"] = json!(now_secs() + 30);
        let cookie = fixture.session_cookie(initial).await;
        let mut refreshed = fixture.access_claims();
        refreshed["exp"] = json!(now_secs() + lifetime);
        fixture.refresh_response(json!({"access_token": mint_jwt(&fixture.key, refreshed), "token_type": "Bearer", "refresh_token": "rotated", "refresh_expires_in": 900})).await;
        let mut tasks = Vec::new();
        for _ in 0..8 {
            let session = fixture.session.clone();
            let id = cookie.strip_prefix("session=").unwrap().to_string();
            tasks.push(tokio::spawn(async move {
                session.authenticate_cookie(&id).await.unwrap()
            }));
        }
        for task in tasks {
            assert!(task.await.unwrap().is_live());
        }
    }
}

#[tokio::test]
async fn browser_mutations_require_origin_while_bearer_calls_do_not() {
    use std::sync::atomic::Ordering;
    let fixture = LoginFixture::new().await;
    let cookie = fixture.session_cookie(fixture.access_claims()).await;
    for origin in [
        None,
        Some("null"),
        Some("https://other.example"),
        Some("https://app.example.evil"),
    ] {
        for target in ["/api/identity", "/logout"] {
            let mut request = Request::builder()
                .method("POST")
                .uri(target)
                .header("cookie", &cookie);
            if let Some(origin) = origin {
                request = request.header("origin", origin);
            }
            let response = fixture
                .app
                .clone()
                .oneshot(request.body(Body::empty()).unwrap())
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::FORBIDDEN);
        }
    }
    assert_eq!(fixture.writes.load(Ordering::SeqCst), 0);
    let response = fixture
        .app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/identity")
                .header("cookie", &cookie)
                .header("origin", "https://app.example")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NO_CONTENT);
    let response = fixture
        .app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/identity")
                .header(
                    "authorization",
                    format!("Bearer {}", mint_jwt(&fixture.key, fixture.access_claims())),
                )
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NO_CONTENT);
    assert_eq!(fixture.writes.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn refresh_cannot_extend_original_lifetime_or_complete_after_expiry() {
    for delayed in [false, true] {
        let fixture = LoginFixture::with_ttl(std::time::Duration::from_millis(300)).await;
        let mut initial = fixture.access_claims();
        initial["exp"] = json!(now_secs() + 30);
        let cookie = fixture.session_cookie(initial).await;
        let id = cookie.strip_prefix("session=").unwrap();
        let response = ResponseTemplate::new(200)
            .set_delay(std::time::Duration::from_millis(if delayed { 500 } else { 0 }))
            .set_body_json(json!({"access_token": mint_jwt(&fixture.key, fixture.access_claims()), "token_type": "Bearer", "refresh_token": "rotated", "refresh_expires_in": 900}));
        Mock::given(method("POST"))
            .and(path("/token"))
            .and(body_string_contains("grant_type=refresh_token"))
            .respond_with(response)
            .expect(1)
            .mount(&fixture.provider)
            .await;
        let result = fixture.session.authenticate_cookie(id).await;
        if delayed {
            assert!(result.is_err());
        } else {
            let access = result.unwrap();
            assert!(access.claims().exp > now_secs() + 60);
            tokio::time::timeout(std::time::Duration::from_secs(1), access.invalidated())
                .await
                .unwrap();
            assert!(!access.is_live());
        }
        assert!(fixture.session.authenticate_cookie(id).await.is_err());
    }
}

#[tokio::test]
async fn dropping_runtime_invalidates_retained_access() {
    let fixture = LoginFixture::new().await;
    let cookie = fixture.session_cookie(fixture.access_claims()).await;
    let access = fixture
        .session
        .authenticate_cookie(cookie.strip_prefix("session=").unwrap())
        .await
        .unwrap();
    drop(fixture);
    assert!(!access.is_live());
    tokio::time::timeout(std::time::Duration::from_secs(1), access.invalidated())
        .await
        .unwrap();
}

impl LoginFixture {
    async fn new() -> Self {
        Self::with_ttl(std::time::Duration::from_secs(8 * 3600)).await
    }

    async fn with_ttl(ttl: std::time::Duration) -> Self {
        let provider = MockServer::start().await;
        let issuer = provider.uri();
        let key = test_key_pair();
        Mock::given(path("/.well-known/openid-configuration"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "issuer": issuer, "authorization_endpoint": format!("{issuer}/authorize"),
                "token_endpoint": format!("{issuer}/token"), "jwks_uri": format!("{issuer}/keys"),
                "end_session_endpoint": format!("{issuer}/logout?tenant=one")
            })))
            .mount(&provider)
            .await;
        Mock::given(path("/keys"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(json!({"keys": [jwk_for_key(&key)]})),
            )
            .mount(&provider)
            .await;
        let mut config = OidcBrowserSessionConfig::new(
            &issuer,
            "web-client",
            "https://app.example/callback",
            SessionKey::from_bytes([42; 32]),
            AudiencePolicy::Require("api".into()),
        );
        config.algorithms = vec![Algorithm::ES256];
        config.clock_skew = std::time::Duration::ZERO;
        config.session_ttl = ttl;
        config.trusted_id_token_audiences = vec!["other".into()];
        let session = OidcBrowserSession::new(config).unwrap();
        let parts = session.browser_layer();
        let api = session.api_layer(AudiencePolicy::Require("api".into()));
        let writes = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let writer = writes.clone();
        let app =
            parts
                .callback_router
                .nest_service(
                    "/api",
                    api.layer(Router::new().route(
                        "/identity",
                        axum::routing::get(|| async { "authenticated" }).post(move || {
                            let writer = writer.clone();
                            async move {
                                writer.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                                StatusCode::NO_CONTENT
                            }
                        }),
                    )),
                )
                .fallback_service(parts.layer.layer(
                    Router::new().route("/review", axum::routing::get(|| async { "review" })),
                ));
        Self {
            provider,
            key,
            app,
            session,
            writes,
        }
    }

    async fn login(&self) -> (String, String, String) {
        let response = self
            .app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/review?tab=files")
                    .header("accept", "text/html")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::FOUND);
        let url = url::Url::parse(response.headers()["location"].to_str().unwrap()).unwrap();
        let pairs: std::collections::HashMap<_, _> = url.query_pairs().into_owned().collect();
        let cookie = response.headers()["set-cookie"]
            .to_str()
            .unwrap()
            .split(';')
            .next()
            .unwrap()
            .to_string();
        (pairs["state"].clone(), pairs["nonce"].clone(), cookie)
    }

    fn access_claims(&self) -> Value {
        json!({"iss": self.provider.uri(), "sub": "owner", "aud": "api", "exp": now_secs() + 300, "iat": now_secs()})
    }

    fn identity_claims(&self, nonce: &str) -> Value {
        json!({"iss": self.provider.uri(), "sub": "owner", "aud": "web-client", "exp": now_secs() + 300, "iat": now_secs(), "nonce": nonce})
    }

    async fn tokens(&self, identity: Option<Value>, access: Value) {
        let mut body = json!({"access_token": mint_jwt(&self.key, access), "token_type": "Bearer", "refresh_token": "refresh", "refresh_expires_in": 900});
        if let Some(identity) = identity {
            body["id_token"] = Value::String(mint_jwt(&self.key, identity));
        }
        self.token_response(body).await;
    }

    async fn token_response(&self, body: Value) {
        Mock::given(method("POST"))
            .and(path("/token"))
            .and(body_string_contains("grant_type=authorization_code"))
            .respond_with(ResponseTemplate::new(200).set_body_json(body))
            .expect(1)
            .mount(&self.provider)
            .await;
    }

    async fn refresh_response(&self, body: Value) {
        Mock::given(method("POST"))
            .and(path("/token"))
            .and(body_string_contains("grant_type=refresh_token"))
            .respond_with(ResponseTemplate::new(200).set_body_json(body))
            .expect(1)
            .mount(&self.provider)
            .await;
    }

    async fn session_cookie(&self, access: Value) -> String {
        let (state, nonce, cookie) = self.login().await;
        self.tokens(Some(self.identity_claims(&nonce)), access)
            .await;
        let response = self.callback(&state, &cookie).await;
        assert_eq!(response.status(), StatusCode::FOUND);
        response
            .headers()
            .get_all("set-cookie")
            .iter()
            .map(|value| value.to_str().unwrap())
            .find(|value| value.starts_with("session="))
            .unwrap()
            .split(';')
            .next()
            .unwrap()
            .to_string()
    }

    async fn callback(&self, state: &str, cookie: &str) -> Response {
        self.app
            .clone()
            .oneshot(
                Request::builder()
                    .uri(format!("/callback?state={state}&code=code"))
                    .header("cookie", cookie)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap()
    }
}

#[tokio::test]
async fn multi_audience_identity_and_access_hash_are_verified() {
    use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
    use sha2::{Digest, Sha256};
    let fixture = LoginFixture::new().await;
    let (state, nonce, cookie) = fixture.login().await;
    let access = mint_jwt(&fixture.key, fixture.access_claims());
    let digest = Sha256::digest(access.as_bytes());
    let mut identity = fixture.identity_claims(&nonce);
    identity["aud"] = json!(["web-client", "other"]);
    identity["azp"] = json!("web-client");
    identity["at_hash"] = json!(URL_SAFE_NO_PAD.encode(&digest[..digest.len() / 2]));
    fixture.token_response(json!({"access_token": access, "token_type": "Bearer", "id_token": mint_jwt(&fixture.key, identity)})).await;
    assert_eq!(
        fixture.callback(&state, &cookie).await.status(),
        StatusCode::FOUND
    );
}

#[tokio::test]
async fn wrong_signature_and_ambiguous_state_cookies_cannot_log_in() {
    let fixture = LoginFixture::new().await;
    let (state, nonce, cookie) = fixture.login().await;
    let duplicate = format!("{cookie}; {cookie}");
    assert_eq!(
        fixture.callback(&state, &duplicate).await.status(),
        StatusCode::BAD_REQUEST
    );
    assert!(!fixture
        .provider
        .received_requests()
        .await
        .unwrap()
        .iter()
        .any(|request| request.url.path() == "/token"));
    let other_key = test_key_pair();
    fixture.token_response(json!({"access_token": mint_jwt(&fixture.key, fixture.access_claims()), "token_type": "Bearer", "refresh_token": "refresh", "id_token": mint_jwt(&other_key, fixture.identity_claims(&nonce))})).await;
    assert_eq!(
        fixture.callback(&state, &cookie).await.status(),
        StatusCode::BAD_GATEWAY
    );
}

#[tokio::test]
async fn provider_error_consumes_state_and_is_not_reflected() {
    let fixture = LoginFixture::new().await;
    let (state, _, cookie) = fixture.login().await;
    let response = fixture.app.clone().oneshot(Request::builder()
        .uri(format!("/callback?state={state}&error=access_denied&error_description=secret-provider-detail"))
        .header("cookie", &cookie).body(Body::empty()).unwrap()).await.unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    assert_eq!(response.headers()["cache-control"], "no-store");
    assert_eq!(
        axum::body::to_bytes(response.into_body(), 1024)
            .await
            .unwrap()
            .as_ref(),
        b"Login failed"
    );
    assert_eq!(
        fixture.callback(&state, &cookie).await.status(),
        StatusCode::BAD_REQUEST
    );
    assert!(!fixture
        .provider
        .received_requests()
        .await
        .unwrap()
        .iter()
        .any(|request| request.url.path() == "/token"));
}

#[tokio::test]
async fn refresh_does_not_issue_invalid_or_different_identity_tokens() {
    for expired in [false, true] {
        for failure in ["subject", "signature", "audience", "nbf", "token-type"] {
            let fixture = LoginFixture::new().await;
            let mut previous = fixture.access_claims();
            previous["exp"] = json!(now_secs() + if expired { -1 } else { 30 });
            // Issue a real session while the original token is valid, then
            // advance its expiry in the server-side record through elapsed time.
            // The expired case is qualified separately below; initial login
            // never admits already expired access tokens.
            if expired {
                previous["exp"] = json!(now_secs() + 2);
            }
            let cookie = fixture.session_cookie(previous).await;
            if expired {
                tokio::time::sleep(std::time::Duration::from_millis(2100)).await;
            }
            let mut next = fixture.access_claims();
            match failure {
                "subject" => next["sub"] = json!("another-owner"),
                "audience" => next["aud"] = json!("other"),
                "nbf" => next["nbf"] = json!(now_secs() + 600),
                _ => (),
            }
            let another_key = test_key_pair();
            let key = if failure == "signature" {
                &another_key
            } else {
                &fixture.key
            };
            let mut body = json!({"access_token": mint_jwt(key, next), "refresh_token": "rotated", "token_type": "Bearer"});
            if failure == "token-type" {
                body["token_type"] = json!("Basic");
            }
            fixture.refresh_response(body).await;
            let response = fixture
                .app
                .clone()
                .oneshot(
                    Request::builder()
                        .uri("/review")
                        .header("cookie", cookie)
                        .body(Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(
                response.status(),
                if expired {
                    StatusCode::UNAUTHORIZED
                } else {
                    StatusCode::OK
                },
                "expired={expired}, {failure}"
            );
            if expired {
                assert!(!response.headers().contains_key("set-cookie"));
            }
        }
    }
}

#[tokio::test]
async fn verified_login_preserves_destination_and_cookie_works_on_shared_api() {
    let fixture = LoginFixture::new().await;
    let (state, nonce, login_cookie) = fixture.login().await;
    assert_ne!(state, nonce);
    fixture
        .tokens(
            Some(fixture.identity_claims(&nonce)),
            fixture.access_claims(),
        )
        .await;
    let response = fixture.callback(&state, &login_cookie).await;
    assert_eq!(response.status(), StatusCode::FOUND);
    assert_eq!(response.headers()["location"], "/review?tab=files");
    assert_eq!(response.headers()["cache-control"], "no-store");
    let cookie = response
        .headers()
        .get_all("set-cookie")
        .iter()
        .map(|value| value.to_str().unwrap())
        .find(|value| value.starts_with("session="))
        .unwrap()
        .split(';')
        .next()
        .unwrap();
    let api = fixture
        .app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/identity")
                .header("cookie", cookie)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(api.status(), StatusCode::OK);
    let replay = fixture.callback(&state, &login_cookie).await;
    assert_eq!(replay.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn invalid_login_claims_and_access_tokens_never_issue_session() {
    for case in [
        "missing-id",
        "nonce",
        "audience",
        "issuer",
        "expired",
        "future-iat",
        "future-nbf",
        "missing-iat",
        "empty-sub",
        "azp",
        "multi-audience",
        "untrusted-audience",
        "malformed-auth-time",
        "malformed-nbf",
        "hash",
        "access-sub",
        "access-exp",
        "access-aud",
        "access-nbf",
    ] {
        let fixture = LoginFixture::new().await;
        let (state, nonce, cookie) = fixture.login().await;
        let mut identity = fixture.identity_claims(&nonce);
        let mut access = fixture.access_claims();
        match case {
            "nonce" => identity["nonce"] = json!("wrong"),
            "audience" => identity["aud"] = json!("other"),
            "issuer" => identity["iss"] = json!("https://other.example"),
            "expired" => identity["exp"] = json!(now_secs() - 1),
            "future-iat" => identity["iat"] = json!(now_secs() + 600),
            "future-nbf" => identity["nbf"] = json!(now_secs() + 600),
            "missing-iat" => {
                identity.as_object_mut().unwrap().remove("iat");
            }
            "empty-sub" => identity["sub"] = json!(""),
            "azp" => identity["azp"] = json!("other"),
            "multi-audience" => identity["aud"] = json!(["web-client", "other"]),
            "untrusted-audience" => {
                identity["aud"] = json!(["web-client", "untrusted"]);
                identity["azp"] = json!("web-client");
            }
            "malformed-auth-time" => identity["auth_time"] = json!("yesterday"),
            "malformed-nbf" => identity["nbf"] = json!("tomorrow"),
            "hash" => identity["at_hash"] = json!("wrong"),
            "access-sub" => access["sub"] = json!("other"),
            "access-exp" => access["exp"] = json!(now_secs() - 1),
            "access-aud" => access["aud"] = json!("other"),
            "access-nbf" => access["nbf"] = json!(now_secs() + 600),
            "missing-id" => (),
            _ => unreachable!(),
        }
        fixture
            .tokens((case != "missing-id").then_some(identity), access)
            .await;
        let response = fixture.callback(&state, &cookie).await;
        assert_eq!(response.status(), StatusCode::BAD_GATEWAY, "{case}");
        assert!(
            response
                .headers()
                .get_all("set-cookie")
                .iter()
                .all(|value| value.to_str().unwrap().starts_with("__auth_state=;")),
            "{case}"
        );
        assert_eq!(
            fixture.callback(&state, &cookie).await.status(),
            StatusCode::BAD_REQUEST,
            "{case}"
        );
    }
}

#[tokio::test]
async fn forged_state_and_concurrent_callback_replay_cannot_redeem_codes() {
    let fixture = LoginFixture::new().await;
    let signed = cli_framework_oidc::browser::auth_state::encode_auth_state(
        &cli_framework_oidc::browser::auth_state::AuthState {
            state: "never-issued".into(),
            verifier: "verifier".into(),
            return_to: "/".into(),
        },
        &cli_framework_oidc::browser::auth_state::derive_hmac_key(&[42; 32]),
    );
    assert_eq!(
        fixture
            .callback("never-issued", &format!("__auth_state={signed}"))
            .await
            .status(),
        StatusCode::BAD_REQUEST
    );
    assert!(!fixture
        .provider
        .received_requests()
        .await
        .unwrap()
        .iter()
        .any(|request| request.url.path() == "/token"));
    let (state, nonce, cookie) = fixture.login().await;
    fixture
        .tokens(
            Some(fixture.identity_claims(&nonce)),
            fixture.access_claims(),
        )
        .await;
    let (first, second) = tokio::join!(
        fixture.callback(&state, &cookie),
        fixture.callback(&state, &cookie)
    );
    let statuses = [first.status(), second.status()];
    assert!(statuses.contains(&StatusCode::FOUND));
    assert!(statuses.contains(&StatusCode::BAD_REQUEST));
}
