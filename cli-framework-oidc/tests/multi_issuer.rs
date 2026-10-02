//! Several trusted issuers and configurable roles/groups claim paths (ADR 0082).
//!
//! Most tests use `TestIssuer` with inline keys (`static_jwks`): no HTTP
//! server, no network, no identity provider. The tests that assert on JWKS
//! traffic serve keys from a `wiremock::MockServer` per issuer instead.

use axum::{response::IntoResponse, routing::get, Router};
use jsonwebtoken::Algorithm;
use serde_json::json;
use tower::{Layer, ServiceExt};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use cli_framework_oidc::server::{
    oidc_validation_layer_multi, AudiencePolicy, JwkSet, OidcClaims, OidcValidationConfig,
    OidcValidationError, OidcValidator, TokenRejection, DEFAULT_ROLES_CLAIM_PATH,
};
use cli_framework_oidc::test_support::{make_cfg, mint_jwt, TestIssuer};
use cli_framework_oidc::OidcConfigError;

const A: &str = "https://issuer-a.test";
const B: &str = "https://issuer-b.test";
const C: &str = "https://issuer-c.test";

fn rejection(err: OidcValidationError) -> TokenRejection {
    match err {
        OidcValidationError::InvalidToken(r) => r,
        other => panic!("expected InvalidToken, got {other:?}"),
    }
}

fn two_issuers() -> (TestIssuer, TestIssuer, OidcValidator) {
    let a = TestIssuer::new(A);
    let b = TestIssuer::new(B);
    let v = OidcValidator::new_multi([a.config(), b.config()]).expect("validator");
    (a, b, v)
}

// ── Routing by issuer ───────────────────────────────────────────────────────

#[tokio::test]
async fn token_from_each_trusted_issuer_validates() {
    let (a, b, v) = two_issuers();
    assert_eq!(v.issuers().collect::<Vec<_>>(), [A, B]);

    let claims = v.validate(&a.mint(json!({"sub": "alice"}))).await.unwrap();
    assert_eq!(claims.sub, "alice");
    assert_eq!(claims.iss, A);

    let claims = v.validate(&b.mint(json!({"sub": "bob"}))).await.unwrap();
    assert_eq!(claims.sub, "bob");
    assert_eq!(claims.iss, B);
}

#[tokio::test]
async fn token_from_untrusted_issuer_is_unknown_issuer() {
    let (_a, _b, v) = two_issuers();
    let c = TestIssuer::new(C);
    let err = v.validate(&c.mint(json!({"sub": "u"}))).await.unwrap_err();
    assert_eq!(rejection(err), TokenRejection::UnknownIssuer);
}

#[tokio::test]
async fn token_without_string_iss_is_unknown_issuer() {
    let (a, _b, v) = two_issuers();
    let no_iss = mint_jwt(&a.key, json!({"sub": "u", "exp": 4_000_000_000_i64}));
    assert_eq!(
        rejection(v.validate(&no_iss).await.unwrap_err()),
        TokenRejection::UnknownIssuer
    );
    let array_iss = a.mint(json!({"sub": "u", "iss": [A]}));
    assert_eq!(
        rejection(v.validate(&array_iss).await.unwrap_err()),
        TokenRejection::UnknownIssuer
    );
}

#[tokio::test]
async fn issuer_must_match_exactly_not_after_normalization() {
    // The configured issuer is normalized; the token's `iss` is compared as is,
    // the same rule the single-issuer `iss` check applies.
    let (a, _b, v) = two_issuers();
    let token = a.mint(json!({"sub": "u", "iss": format!("{A}/")}));
    assert_eq!(
        rejection(v.validate(&token).await.unwrap_err()),
        TokenRejection::UnknownIssuer
    );
}

#[tokio::test]
async fn undecodable_payload_is_malformed() {
    let (a, _b, v) = two_issuers();
    let token = a.mint(json!({"sub": "u"}));
    let mut parts: Vec<&str> = token.split('.').collect();
    parts[1] = "!!not-base64!!";
    let broken = parts.join(".");
    assert_eq!(
        rejection(v.validate(&broken).await.unwrap_err()),
        TokenRejection::Malformed
    );
}

// ── Keys never cross issuers ────────────────────────────────────────────────

#[tokio::test]
async fn iss_of_a_signed_by_b_is_rejected() {
    let (_a, b, v) = two_issuers();
    // B's key, B's kid, but claiming A: A's key set has no such kid.
    let forged = b.mint(json!({"sub": "u", "iss": A}));
    assert_eq!(
        rejection(v.validate(&forged).await.unwrap_err()),
        TokenRejection::UnknownKey
    );
}

#[tokio::test]
async fn iss_of_a_signed_by_b_with_a_colliding_kid_fails_signature() {
    // Both issuers publish the same kid; the token must still be checked
    // against A's key only.
    let a = TestIssuer::with_kid(A, "shared-kid");
    let b = TestIssuer::with_kid(B, "shared-kid");
    let v = OidcValidator::new_multi([a.config(), b.config()]).unwrap();
    let forged = b.mint(json!({"sub": "u", "iss": A}));
    assert_eq!(
        rejection(v.validate(&forged).await.unwrap_err()),
        TokenRejection::InvalidSignature
    );
    // Sanity: each issuer's own token still validates.
    assert!(v.validate(&a.mint(json!({"sub": "u"}))).await.is_ok());
    assert!(v.validate(&b.mint(json!({"sub": "u"}))).await.is_ok());
}

async fn mount_jwks(server: &MockServer, issuer: &TestIssuer) {
    Mock::given(method("GET"))
        .and(path("/jwks"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "keys": [issuer.jwk()] })))
        .mount(server)
        .await;
}

#[tokio::test]
async fn unknown_issuer_triggers_no_discovery_or_jwks_fetch() {
    let server_a = MockServer::start().await;
    let server_b = MockServer::start().await;
    // A pins its jwks_uri; B relies on discovery. Neither may be contacted.
    let cfg_a = OidcValidationConfig {
        algorithms: vec![Algorithm::ES256],
        ..make_cfg(&server_a.uri())
    };
    let cfg_b = OidcValidationConfig {
        algorithms: vec![Algorithm::ES256],
        ..OidcValidationConfig::new(server_b.uri(), AudiencePolicy::Unchecked)
    };
    let v = OidcValidator::new_multi([cfg_a, cfg_b]).unwrap();

    let c = TestIssuer::new(C);
    for _ in 0..5 {
        let err = v.validate(&c.mint(json!({"sub": "u"}))).await.unwrap_err();
        assert_eq!(rejection(err), TokenRejection::UnknownIssuer);
    }
    assert!(server_a.received_requests().await.unwrap().is_empty());
    assert!(server_b.received_requests().await.unwrap().is_empty());
}

#[tokio::test]
async fn each_issuer_fetches_only_its_own_jwks() {
    let server_a = MockServer::start().await;
    let server_b = MockServer::start().await;
    let a = TestIssuer::new(&server_a.uri());
    let b = TestIssuer::new(&server_b.uri());
    mount_jwks(&server_a, &a).await;
    mount_jwks(&server_b, &b).await;
    let cfg = |s: &MockServer| OidcValidationConfig {
        algorithms: vec![Algorithm::ES256],
        ..make_cfg(&s.uri())
    };
    let v = OidcValidator::new_multi([cfg(&server_a), cfg(&server_b)]).unwrap();

    // Claims A, signed by B with B's kid: only A's JWKS is consulted, and it
    // does not contain B's kid.
    let forged = b.mint(json!({"sub": "u", "iss": a.issuer}));
    assert_eq!(
        rejection(v.validate(&forged).await.unwrap_err()),
        TokenRejection::UnknownKey
    );
    assert_eq!(server_a.received_requests().await.unwrap().len(), 1);
    assert!(server_b.received_requests().await.unwrap().is_empty());

    // B's genuine token is served from B's own JWKS.
    let claims = v.validate(&b.mint(json!({"sub": "u"}))).await.unwrap();
    assert_eq!(claims.iss, b.issuer);
    assert_eq!(server_b.received_requests().await.unwrap().len(), 1);
}

// ── Per-issuer settings ─────────────────────────────────────────────────────

#[tokio::test]
async fn audience_is_enforced_per_issuer() {
    let a = TestIssuer::new(A);
    let b = TestIssuer::new(B);
    let v = OidcValidator::new_multi([
        OidcValidationConfig {
            audience: AudiencePolicy::Require("api-a".into()),
            ..a.config()
        },
        OidcValidationConfig {
            audience: AudiencePolicy::Require("api-b".into()),
            ..b.config()
        },
    ])
    .unwrap();

    assert!(v
        .validate(&a.mint(json!({"sub": "u", "aud": "api-a"})))
        .await
        .is_ok());
    assert!(v
        .validate(&b.mint(json!({"sub": "u", "aud": "api-b"})))
        .await
        .is_ok());
    // A token from A carrying B's audience is not accepted by A.
    let err = v
        .validate(&a.mint(json!({"sub": "u", "aud": "api-b"})))
        .await
        .unwrap_err();
    assert_eq!(rejection(err), TokenRejection::InvalidAudience);
}

#[tokio::test]
async fn algorithms_are_enforced_per_issuer() {
    let a = TestIssuer::new(A);
    let b = TestIssuer::new(B);
    let v = OidcValidator::new_multi([
        a.config(),
        OidcValidationConfig {
            algorithms: vec![Algorithm::RS256],
            ..b.config()
        },
    ])
    .unwrap();
    assert!(v.validate(&a.mint(json!({"sub": "u"}))).await.is_ok());
    let err = v.validate(&b.mint(json!({"sub": "u"}))).await.unwrap_err();
    assert_eq!(rejection(err), TokenRejection::UnsupportedAlgorithm);
}

#[tokio::test]
async fn expired_token_from_a_trusted_issuer_is_expired() {
    let (a, _b, v) = two_issuers();
    let token = a.mint(json!({"sub": "u", "exp": 1_000_000_000_i64}));
    assert_eq!(
        rejection(v.validate(&token).await.unwrap_err()),
        TokenRejection::Expired
    );
}

// ── Claim paths ─────────────────────────────────────────────────────────────

#[tokio::test]
async fn default_roles_path_is_realm_access_roles_and_groups_is_empty() {
    let a = TestIssuer::new(A);
    assert_eq!(a.config().roles_claim_path, DEFAULT_ROLES_CLAIM_PATH);
    let v = OidcValidator::new(a.config()).unwrap();
    let claims = v
        .validate(&a.mint(json!({
            "sub": "u",
            "realm_access": {"roles": ["admin", "user"]},
            "groups": ["ignored-without-a-path"],
        })))
        .await
        .unwrap();
    assert_eq!(claims.roles, ["admin", "user"]);
    assert!(claims.groups.is_empty());
}

#[tokio::test]
async fn roles_read_from_a_nested_custom_path() {
    let a = TestIssuer::new(A);
    let b = TestIssuer::new(B);
    let v = OidcValidator::new_multi([a.config().with_roles_claim_path("org.roles"), b.config()])
        .unwrap();

    let claims = v
        .validate(&a.mint(json!({
            "sub": "u",
            "org": {"roles": ["x:viewer"]},
            "realm_access": {"roles": ["not-read-for-a"]},
        })))
        .await
        .unwrap();
    assert_eq!(claims.roles, ["x:viewer"]);

    // B keeps the default path.
    let claims = v
        .validate(&b.mint(json!({"sub": "u", "realm_access": {"roles": ["b-role"]}})))
        .await
        .unwrap();
    assert_eq!(claims.roles, ["b-role"]);
}

#[tokio::test]
async fn groups_read_from_the_configured_path() {
    let a = TestIssuer::new(A);
    let v = OidcValidator::new_multi([a.config().with_groups_claim_path("groups")]).unwrap();
    let claims = v
        .validate(&a.mint(json!({"sub": "u", "groups": ["/team-a", "/team-b", 3]})))
        .await
        .unwrap();
    assert_eq!(claims.groups, ["/team-a", "/team-b"]);

    // A single string is a one-element list; a missing claim is an empty list.
    let claims = v
        .validate(&a.mint(json!({"sub": "u", "groups": "/solo"})))
        .await
        .unwrap();
    assert_eq!(claims.groups, ["/solo"]);
    let claims = v.validate(&a.mint(json!({"sub": "u"}))).await.unwrap();
    assert!(claims.groups.is_empty());
    assert!(claims.roles.is_empty());
}

#[tokio::test]
async fn escaped_dot_selects_a_key_containing_a_dot() {
    let a = TestIssuer::new(A);
    let v = OidcValidator::new(
        a.config()
            .with_roles_claim_path(r"https://example\.com/claims.roles")
            .with_groups_claim_path(r"https://example\.com/groups"),
    )
    .unwrap();
    let claims = v
        .validate(&a.mint(json!({
            "sub": "u",
            "https://example.com/claims": {"roles": ["dotted-role"]},
            "https://example.com/groups": ["dotted-group"],
        })))
        .await
        .unwrap();
    assert_eq!(claims.roles, ["dotted-role"]);
    assert_eq!(claims.groups, ["dotted-group"]);
}

// ── Configuration errors ────────────────────────────────────────────────────

#[test]
fn duplicate_issuers_after_normalization_are_rejected() {
    let a = TestIssuer::new(A);
    let mut dup = a.config();
    dup.issuer_url = "HTTPS://ISSUER-A.TEST:443/".into();
    let err = OidcValidator::new_multi([a.config(), dup])
        .err()
        .expect("duplicate issuer");
    assert!(matches!(err, OidcConfigError::DuplicateIssuer(ref i) if i == A));
}

#[test]
fn empty_issuer_list_is_rejected() {
    let err = OidcValidator::new_multi(Vec::new()).err().expect("empty");
    assert!(matches!(err, OidcConfigError::MissingField("issuers")));
}

#[test]
fn invalid_claim_paths_are_rejected_at_construction() {
    let a = TestIssuer::new(A);
    for (roles, groups) in [
        ("", None),
        ("a..b", None),
        (r"a\x", None),
        ("ok", Some("x.")),
    ] {
        let mut cfg = a.config().with_roles_claim_path(roles);
        cfg.groups_claim_path = groups.map(String::from);
        let err = OidcValidator::new(cfg).err().expect("invalid path");
        assert!(matches!(err, OidcConfigError::InvalidClaimPath(_)), "{err}");
    }
}

#[test]
fn static_jwks_conflicts_with_jwks_uri_and_must_hold_public_keys() {
    let a = TestIssuer::new(A);

    let mut both = a.config();
    both.jwks_uri = Some(format!("{A}/jwks"));
    assert!(matches!(
        OidcValidator::new(both).err(),
        Some(OidcConfigError::InvalidJwks(_))
    ));

    let empty: JwkSet = serde_json::from_value(json!({"keys": []})).unwrap();
    assert!(matches!(
        OidcValidator::new(a.config().with_static_jwks(empty)).err(),
        Some(OidcConfigError::InvalidJwks(_))
    ));

    let symmetric: JwkSet =
        serde_json::from_value(json!({"keys": [{"kty": "oct", "k": "c2VjcmV0", "kid": "s"}]}))
            .unwrap();
    assert!(matches!(
        OidcValidator::new(a.config().with_static_jwks(symmetric)).err(),
        Some(OidcConfigError::InvalidJwks(_))
    ));
}

// ── Single-issuer behaviour is unchanged ────────────────────────────────────

#[tokio::test]
async fn single_issuer_validator_still_reports_invalid_issuer() {
    // `new` keeps its behaviour: no issuer routing, so a foreign `iss` signed
    // with the trusted key is InvalidIssuer, not UnknownIssuer.
    let a = TestIssuer::new(A);
    let v = OidcValidator::new(a.config()).unwrap();
    let token = a.mint(json!({"sub": "u", "iss": C}));
    assert_eq!(
        rejection(v.validate(&token).await.unwrap_err()),
        TokenRejection::InvalidIssuer
    );
}

// ── HTTP layer and Authorization header ─────────────────────────────────────

async fn send(app: Router, token: &str) -> axum::response::Response {
    let req = axum::http::Request::builder()
        .uri("/protected")
        .header("authorization", format!("Bearer {token}"))
        .body(axum::body::Body::empty())
        .unwrap();
    app.oneshot(req).await.unwrap()
}

fn app_with(layer: cli_framework_oidc::server::BoxedOidcLayer) -> Router {
    async fn protected(claims: OidcClaims) -> impl IntoResponse {
        axum::Json(json!({"iss": claims.iss, "roles": claims.roles, "groups": claims.groups}))
    }
    let inner = Router::new().route("/protected", get(protected));
    Router::new().fallback_service(layer.layer(inner))
}

#[tokio::test]
async fn multi_issuer_layer_accepts_both_issuers_and_rejects_others() {
    let a = TestIssuer::new(A);
    let b = TestIssuer::new(B);
    let c = TestIssuer::new(C);
    let layer = oidc_validation_layer_multi([
        a.config().with_groups_claim_path("groups"),
        b.config().with_roles_claim_path("org.roles"),
    ])
    .unwrap();
    let app = app_with(layer);

    let resp = send(app.clone(), &a.mint(json!({"sub": "u", "groups": ["g"]}))).await;
    assert_eq!(resp.status(), axum::http::StatusCode::OK);
    let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    let body: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(body, json!({"iss": A, "roles": [], "groups": ["g"]}));

    let resp = send(
        app.clone(),
        &b.mint(json!({"sub": "u", "org": {"roles": ["r"]}})),
    )
    .await;
    assert_eq!(resp.status(), axum::http::StatusCode::OK);
    let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    let body: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(body, json!({"iss": B, "roles": ["r"], "groups": []}));

    let resp = send(app, &c.mint(json!({"sub": "u"}))).await;
    assert_eq!(resp.status(), axum::http::StatusCode::UNAUTHORIZED);
    assert_eq!(
        resp.headers()["www-authenticate"],
        "Bearer error=\"invalid_token\", error_description=\"unknown_issuer\""
    );
}

#[tokio::test]
async fn validator_layer_and_authorization_header_work_for_multi_issuer() {
    let (a, b, v) = two_issuers();
    let header = format!("Bearer {}", b.mint(json!({"sub": "u"})));
    assert_eq!(
        v.validate_authorization(Some(&header)).await.unwrap().iss,
        B
    );
    assert_eq!(
        v.validate_authorization(None).await.unwrap_err(),
        OidcValidationError::MissingToken
    );

    let resp = send(app_with(v.layer()), &a.mint(json!({"sub": "u"}))).await;
    assert_eq!(resp.status(), axum::http::StatusCode::OK);
}

#[tokio::test]
async fn jwks_unavailable_retry_after_comes_from_the_selected_issuer() {
    let server_b = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/jwks"))
        .respond_with(ResponseTemplate::new(500))
        .mount(&server_b)
        .await;
    let a = TestIssuer::new(A);
    let b = TestIssuer::new(&server_b.uri());
    let layer = oidc_validation_layer_multi([
        OidcValidationConfig {
            min_refetch_interval: std::time::Duration::from_secs(11),
            ..a.config()
        },
        OidcValidationConfig {
            algorithms: vec![Algorithm::ES256],
            min_refetch_interval: std::time::Duration::from_secs(7),
            ..make_cfg(&server_b.uri())
        },
    ])
    .unwrap();

    let resp = send(app_with(layer), &b.mint(json!({"sub": "u"}))).await;
    assert_eq!(resp.status(), axum::http::StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(resp.headers()["retry-after"], "7");
}
