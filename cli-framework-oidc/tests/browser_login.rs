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
    matchers::{method, path},
    Mock, MockServer, ResponseTemplate,
};

struct LoginFixture {
    provider: MockServer,
    key: TestKeyPair,
    app: Router,
}

impl LoginFixture {
    async fn new() -> Self {
        let provider = MockServer::start().await;
        let issuer = provider.uri();
        let key = test_key_pair();
        Mock::given(path("/.well-known/openid-configuration"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "issuer": issuer, "authorization_endpoint": format!("{issuer}/authorize"),
                "token_endpoint": format!("{issuer}/token"), "jwks_uri": format!("{issuer}/keys")
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
        let session = OidcBrowserSession::new(config).unwrap();
        let parts = session.browser_layer();
        let api = session.api_layer(AudiencePolicy::Require("api".into()));
        let app =
            parts
                .callback_router
                .nest_service(
                    "/api",
                    api.layer(Router::new().route(
                        "/identity",
                        axum::routing::get(|| async { "authenticated" }),
                    )),
                )
                .fallback_service(parts.layer.layer(
                    Router::new().route("/review", axum::routing::get(|| async { "review" })),
                ));
        Self { provider, key, app }
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
            .respond_with(ResponseTemplate::new(200).set_body_json(body))
            .expect(1)
            .mount(&self.provider)
            .await;
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
            let cookie = cli_framework_oidc::browser::cookie::encrypt_cookie(
                &[42; 32],
                &mint_jwt(&fixture.key, previous),
                "refresh",
                now_secs() + 900,
            )
            .unwrap();
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
            fixture.token_response(body).await;
            let response = fixture
                .app
                .clone()
                .oneshot(
                    Request::builder()
                        .uri("/review")
                        .header("cookie", format!("session={cookie}"))
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
            assert!(
                !response.headers().contains_key("set-cookie"),
                "expired={expired}, {failure}"
            );
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
