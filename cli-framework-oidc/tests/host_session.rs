//! Host sessions against a mock realm (wiremock: discovery, JWKS, token and
//! end-session endpoints), driven through the router and `resolve`.

use cli_framework::axum::body::Body;
use cli_framework::axum::http::{header, HeaderMap, HeaderValue, Request, StatusCode};
use cli_framework::axum::response::Response;
use cli_framework_oidc::host_session::{
    ClientSecret, Clock, EndReason, HostSessionConfig, HostSessions, Resolution, SessionKey,
};
use cli_framework_oidc::test_support::{now_secs, TestIssuer};
use jsonwebtoken::Algorithm;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::Arc;
use tower::ServiceExt;
use wiremock::matchers::{body_string_contains, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

const ORIGIN: &str = "http://localhost:8080";
const BINDING: &str = "web-meridis.faseinfra.net";

struct Realm {
    server: MockServer,
    issuer: TestIssuer,
}

impl Realm {
    async fn start() -> Self {
        let server = MockServer::start().await;
        let issuer = TestIssuer::new(&server.uri());
        let uri = server.uri();
        Mock::given(method("GET"))
            .and(path("/.well-known/openid-configuration"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "issuer": uri,
                "authorization_endpoint": format!("{uri}/auth"),
                "token_endpoint": format!("{uri}/token"),
                "end_session_endpoint": format!("{uri}/logout"),
                "jwks_uri": format!("{uri}/jwks"),
            })))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/jwks"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(json!({ "keys": [issuer.jwk()] })),
            )
            .mount(&server)
            .await;
        Self { server, issuer }
    }

    fn access(&self, ttl: i64) -> String {
        self.issuer.mint(json!({
            "sub": "user-1", "aud": "account", "name": "Ana Souza",
            "organization": {"clinic-a": {}}, "exp": now_secs() + ttl,
            "jti": rand_jti(),
        }))
    }

    fn id(&self, nonce: &str) -> String {
        self.issuer
            .mint(json!({ "sub": "user-1", "aud": "meridis-apps-host", "nonce": nonce }))
    }

    /// Answers the next authorization-code exchange with these tokens.
    async fn on_code(&self, body: Value) {
        Mock::given(method("POST"))
            .and(path("/token"))
            .and(body_string_contains("grant_type=authorization_code"))
            .respond_with(ResponseTemplate::new(200).set_body_json(body))
            .mount(&self.server)
            .await;
    }

    async fn on_refresh(&self, resp: ResponseTemplate) {
        Mock::given(method("POST"))
            .and(path("/token"))
            .and(body_string_contains("grant_type=refresh_token"))
            .respond_with(resp)
            .mount(&self.server)
            .await;
    }

    async fn bodies(&self, p: &str) -> Vec<String> {
        self.server
            .received_requests()
            .await
            .unwrap()
            .into_iter()
            .filter(|r| r.url.path() == p)
            .map(|r| String::from_utf8(r.body).unwrap())
            .collect()
    }
}

fn rand_jti() -> String {
    use std::sync::atomic::AtomicU64;
    static N: AtomicU64 = AtomicU64::new(0);
    format!("jti-{}", N.fetch_add(1, Ordering::Relaxed))
}

/// A clock at real time plus a movable offset.
#[derive(Clone)]
struct TestClock(Arc<AtomicI64>);

impl TestClock {
    fn new() -> Self {
        Self(Arc::new(AtomicI64::new(0)))
    }
    fn advance(&self, secs: i64) {
        self.0.fetch_add(secs, Ordering::SeqCst);
    }
    fn clock(&self) -> Clock {
        let off = Arc::clone(&self.0);
        Clock::from_fn(move || now_secs() + off.load(Ordering::SeqCst))
    }
}

fn config(realm: &Realm, clock: &TestClock) -> HostSessionConfig {
    let mut cfg = HostSessionConfig::new(
        realm.server.uri(),
        "meridis-apps-host",
        format!("{ORIGIN}/_host/callback"),
        SessionKey::from_bytes([7; 32]),
        BINDING,
    );
    cfg.client_secret = Some(ClientSecret::new("s3cret"));
    cfg.algorithms = vec![Algorithm::ES256];
    cfg.session_claims.push("organization".into());
    cfg.clock = clock.clock();
    cfg
}

async fn call(s: &HostSessions, req: Request<Body>) -> Response {
    s.router().oneshot(req).await.unwrap()
}

fn get(uri: &str, cookies: &[String]) -> Request<Body> {
    let mut b = Request::get(uri);
    if !cookies.is_empty() {
        b = b.header(header::COOKIE, cookies.join("; "));
    }
    b.body(Body::empty()).unwrap()
}

fn set_cookies(resp: &Response) -> Vec<String> {
    resp.headers()
        .get_all(header::SET_COOKIE)
        .iter()
        .map(|v| v.to_str().unwrap().to_string())
        .collect()
}

/// The `name=value` pair of a Set-Cookie for `name`, if any.
fn cookie_pair(resp: &Response, name: &str) -> Option<String> {
    set_cookies(resp)
        .into_iter()
        .find(|c| c.starts_with(&format!("{name}=")))
        .map(|c| c.split(';').next().unwrap().to_string())
}

fn location(resp: &Response) -> String {
    resp.headers()[header::LOCATION]
        .to_str()
        .unwrap()
        .to_string()
}

fn query(url: &str) -> HashMap<String, String> {
    url::Url::parse(url)
        .unwrap()
        .query_pairs()
        .into_owned()
        .collect()
}

fn cookie_headers(pair: &str) -> HeaderMap {
    let mut h = HeaderMap::new();
    h.insert(header::COOKIE, HeaderValue::from_str(pair).unwrap());
    h
}

async fn body_json(resp: Response) -> Value {
    let bytes = cli_framework::axum::body::to_bytes(resp.into_body(), 1 << 20)
        .await
        .unwrap();
    serde_json::from_slice(&bytes).unwrap()
}

/// Runs `/login` then `/callback`; returns the callback response.
async fn sign_in_with(
    s: &HostSessions,
    realm: &Realm,
    tokens: impl FnOnce(&str) -> Value,
) -> Response {
    let login = call(s, get("/_host/login?return_to=/patients%3Fx%3D1", &[])).await;
    assert_eq!(login.status(), StatusCode::FOUND);
    let q = query(&location(&login));
    let sign_in = cookie_pair(&login, "__Host-sign-in").unwrap();
    realm.on_code(tokens(&q["nonce"])).await;
    call(
        s,
        get(
            &format!("/_host/callback?code=abc&state={}", q["state"]),
            &[sign_in],
        ),
    )
    .await
}

/// Signs in with an access token living `at_ttl` seconds; returns the cookie pair.
async fn sign_in(s: &HostSessions, realm: &Realm, at_ttl: i64) -> String {
    let at = realm.access(at_ttl);
    let resp = sign_in_with(s, realm, |nonce| {
        json!({ "access_token": at, "refresh_token": "rt-1", "id_token": realm.id(nonce),
                "refresh_expires_in": 36000, "expires_in": at_ttl })
    })
    .await;
    assert_eq!(resp.status(), StatusCode::SEE_OTHER);
    cookie_pair(&resp, "__Host-session").expect("session cookie")
}

#[tokio::test]
async fn login_redirects_with_pkce_state_and_nonce() {
    let realm = Realm::start().await;
    let s = HostSessions::new(config(&realm, &TestClock::new())).unwrap();
    let resp = call(&s, get("/_host/login?return_to=/a", &[])).await;
    assert_eq!(resp.status(), StatusCode::FOUND);
    let loc = location(&resp);
    assert!(loc.starts_with(&format!("{}/auth?", realm.server.uri())));
    let q = query(&loc);
    assert_eq!(q["response_type"], "code");
    assert_eq!(q["client_id"], "meridis-apps-host");
    assert_eq!(q["redirect_uri"], format!("{ORIGIN}/_host/callback"));
    assert_eq!(q["scope"], "openid");
    assert_eq!(q["code_challenge_method"], "S256");
    for k in ["state", "nonce", "code_challenge"] {
        assert!(q[k].len() >= 43, "{k} is random and long");
    }
    let c = set_cookies(&resp).join("\n");
    assert!(c.contains("__Host-sign-in="));
    assert!(c.contains("Path=/; Secure; HttpOnly; SameSite=Lax; Max-Age=600"));
    assert_eq!(resp.headers()[header::CACHE_CONTROL], "no-store");

    for bad in [
        "//evil.example",
        "https://evil.example/",
        "%5Cevil",
        "relative",
        // Each decodes to a path a browser reads as `//evil.example`.
        "/%5Cevil.example",
        "/%09/evil.example",
    ] {
        let resp = call(&s, get(&format!("/_host/login?return_to={bad}"), &[])).await;
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST, "{bad}");
    }
}

#[tokio::test]
async fn callback_sets_a_sealed_session_cookie_without_max_age() {
    let realm = Realm::start().await;
    let s = HostSessions::new(config(&realm, &TestClock::new())).unwrap();
    let at = realm.access(300);
    let resp = sign_in_with(&s, &realm, |nonce| {
        json!({ "access_token": at, "refresh_token": "rt-1", "id_token": realm.id(nonce),
                "refresh_expires_in": 1800 })
    })
    .await;
    assert_eq!(resp.status(), StatusCode::SEE_OTHER);
    assert_eq!(location(&resp), "/patients?x=1");
    let cookies = set_cookies(&resp);
    let session = cookies
        .iter()
        .find(|c| c.starts_with("__Host-session="))
        .unwrap();
    assert!(
        session.ends_with("; Path=/; Secure; HttpOnly; SameSite=Lax"),
        "{session}"
    );
    assert!(!session.contains("Max-Age"));
    assert!(!session.contains(&at), "the token is sealed, not visible");
    assert!(cookies
        .iter()
        .any(|c| c.starts_with("__Host-sign-in=;") && c.ends_with("Max-Age=0")));

    let body = realm.bodies("/token").await.pop().unwrap();
    for part in [
        "client_id=meridis-apps-host",
        "client_secret=s3cret",
        "code=abc",
        "code_verifier=",
    ] {
        assert!(body.contains(part), "{part} in {body}");
    }

    let pair = session.split(';').next().unwrap();
    match s.resolve(&cookie_headers(pair)).await {
        Resolution::Active(a) => {
            assert_eq!(a.access_token(), at);
            assert_eq!(a.sid().len(), 32);
            assert_eq!(a.claims()["sub"], "user-1");
            assert!(a.set_cookie().is_none(), "no rewrite right after sign-in");
        }
        Resolution::Ended { reason, .. } => panic!("ended: {reason:?}"),
    }
}

#[tokio::test]
async fn callback_refuses_forged_or_stale_sign_ins() {
    let realm = Realm::start().await;
    let clock = TestClock::new();
    let s = HostSessions::new(config(&realm, &clock)).unwrap();

    // No sign-in cookie at all (a login CSRF or a replayed callback URL).
    let resp = call(&s, get("/_host/callback?code=abc&state=x", &[])).await;
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);

    // State mismatch.
    let login = call(&s, get("/_host/login", &[])).await;
    let sign_in = cookie_pair(&login, "__Host-sign-in").unwrap();
    let resp = call(
        &s,
        get("/_host/callback?code=abc&state=other", &[sign_in.clone()]),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    assert!(cookie_pair(&resp, "__Host-session").is_none());

    // Realm error, state correct.
    let q = query(&location(&login));
    let resp = call(
        &s,
        get(
            &format!("/_host/callback?error=access_denied&state={}", q["state"]),
            &[sign_in.clone()],
        ),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);

    // Sign-in older than ten minutes.
    clock.advance(601);
    let resp = call(
        &s,
        get(
            &format!("/_host/callback?code=abc&state={}", q["state"]),
            &[sign_in],
        ),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn callback_refuses_a_wrong_nonce() {
    let realm = Realm::start().await;
    let s = HostSessions::new(config(&realm, &TestClock::new())).unwrap();
    let at = realm.access(300);
    let id = realm.id("someone-elses-nonce");
    let resp = sign_in_with(&s, &realm, |_| {
        json!({ "access_token": at, "refresh_token": "rt", "id_token": id, "refresh_expires_in": 1800 })
    })
    .await;
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    assert!(cookie_pair(&resp, "__Host-session").is_none());
}

#[tokio::test]
async fn callback_refuses_an_id_token_for_another_client_or_subject() {
    let realm = Realm::start().await;
    let s = HostSessions::new(config(&realm, &TestClock::new())).unwrap();
    let at = realm.access(300);
    let resp = sign_in_with(&s, &realm, |nonce| {
        let id = realm.issuer.mint(json!({ "sub": "user-1", "aud": "meridis-frontend", "nonce": nonce }));
        json!({ "access_token": at, "refresh_token": "rt", "id_token": id, "refresh_expires_in": 1800 })
    })
    .await;
    assert_eq!(resp.status(), StatusCode::BAD_GATEWAY);

    let realm = Realm::start().await;
    let s = HostSessions::new(config(&realm, &TestClock::new())).unwrap();
    let at = realm.access(300);
    let resp = sign_in_with(&s, &realm, |nonce| {
        let id = realm.issuer.mint(json!({ "sub": "user-2", "aud": "meridis-apps-host", "nonce": nonce }));
        json!({ "access_token": at, "refresh_token": "rt", "id_token": id, "refresh_expires_in": 1800 })
    })
    .await;
    assert_eq!(resp.status(), StatusCode::BAD_GATEWAY);

    // An access token signed by another key.
    let realm = Realm::start().await;
    let s = HostSessions::new(config(&realm, &TestClock::new())).unwrap();
    let forger = TestIssuer::with_kid(&realm.server.uri(), "forged");
    let resp = sign_in_with(&s, &realm, |nonce| {
        json!({ "access_token": forger.mint(json!({"sub": "user-1"})), "refresh_token": "rt",
                "id_token": realm.id(nonce), "refresh_expires_in": 1800 })
    })
    .await;
    assert_eq!(resp.status(), StatusCode::BAD_GATEWAY);
}

#[tokio::test]
async fn a_cookie_bound_elsewhere_or_tampered_is_refused() {
    let realm = Realm::start().await;
    let clock = TestClock::new();
    let s = HostSessions::new(config(&realm, &clock)).unwrap();
    let pair = sign_in(&s, &realm, 300).await;

    let mut other = config(&realm, &clock);
    other.binding = "web-other.example".into();
    let other = HostSessions::new(other).unwrap();
    match other.resolve(&cookie_headers(&pair)).await {
        Resolution::Ended {
            reason,
            clear_cookie,
        } => {
            assert_eq!(reason, EndReason::BindingMismatch);
            assert!(clear_cookie
                .unwrap()
                .to_str()
                .unwrap()
                .contains("Max-Age=0"));
        }
        Resolution::Active(_) => panic!("a cookie bound elsewhere was accepted"),
    }

    let mut tampered = pair.clone();
    let last = tampered.pop().unwrap();
    tampered.push(if last == 'A' { 'B' } else { 'A' });
    assert!(matches!(
        s.resolve(&cookie_headers(&tampered)).await,
        Resolution::Ended {
            reason: EndReason::Invalid,
            ..
        }
    ));

    let mut other_key = config(&realm, &clock);
    other_key.session_key = SessionKey::from_bytes([8; 32]);
    assert!(matches!(
        HostSessions::new(other_key)
            .unwrap()
            .resolve(&cookie_headers(&pair))
            .await,
        Resolution::Ended {
            reason: EndReason::Invalid,
            ..
        }
    ));

    assert!(matches!(
        s.resolve(&HeaderMap::new()).await,
        Resolution::Ended {
            reason: EndReason::NoSession,
            clear_cookie: None
        }
    ));
}

#[tokio::test]
async fn idle_ends_the_session_and_the_stamp_moves_after_a_tenth() {
    let realm = Realm::start().await;
    let clock = TestClock::new();
    let s = HostSessions::new(config(&realm, &clock)).unwrap();
    let first = sign_in(&s, &realm, 7200).await;

    clock.advance(120);
    let Resolution::Active(a) = s.resolve(&cookie_headers(&first)).await else {
        panic!()
    };
    assert!(
        a.set_cookie().is_none(),
        "2 minutes in: under a tenth of 30, no rewrite"
    );

    clock.advance(120);
    let Resolution::Active(a) = s.resolve(&cookie_headers(&first)).await else {
        panic!()
    };
    let rewritten = a.set_cookie().expect("4 minutes in: stamp rewritten");
    let second = rewritten
        .to_str()
        .unwrap()
        .split(';')
        .next()
        .unwrap()
        .to_string();
    assert!(!rewritten.to_str().unwrap().contains("Max-Age"));
    let sid = a.sid().to_string();

    clock.advance(29 * 60);
    let Resolution::Active(a) = s.resolve(&cookie_headers(&second)).await else {
        panic!("29 minutes after the new stamp is still live")
    };
    assert_eq!(a.sid(), sid, "the session id survives a rewrite");
    match s.resolve(&cookie_headers(&first)).await {
        Resolution::Ended {
            reason,
            clear_cookie,
        } => {
            assert_eq!(reason, EndReason::Idle);
            assert!(clear_cookie.is_some());
        }
        Resolution::Active(_) => panic!("33 minutes idle on the old stamp"),
    }
    let resp = call(&s, get("/_host/session", &[first])).await;
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    assert!(set_cookies(&resp)[0].contains("Max-Age=0"));
}

#[tokio::test]
async fn the_session_ends_with_the_refresh_token() {
    let realm = Realm::start().await;
    let clock = TestClock::new();
    let s = HostSessions::new(config(&realm, &clock)).unwrap();
    let at = realm.access(7200);
    let resp = sign_in_with(&s, &realm, |nonce| {
        json!({ "access_token": at, "refresh_token": "rt", "id_token": realm.id(nonce),
                "refresh_expires_in": 600 })
    })
    .await;
    let pair = cookie_pair(&resp, "__Host-session").unwrap();
    clock.advance(601);
    assert!(matches!(
        s.resolve(&cookie_headers(&pair)).await,
        Resolution::Ended {
            reason: EndReason::Expired,
            ..
        }
    ));
}

#[tokio::test]
async fn an_access_token_near_expiry_is_refreshed_with_the_client_secret() {
    let realm = Realm::start().await;
    let s = HostSessions::new(config(&realm, &TestClock::new())).unwrap();
    let pair = sign_in(&s, &realm, 30).await;
    let fresh = realm.access(300);
    realm
        .on_refresh(ResponseTemplate::new(200).set_body_json(json!({
            "access_token": fresh, "refresh_token": "rt-2", "refresh_expires_in": 1800,
        })))
        .await;
    let Resolution::Active(a) = s.resolve(&cookie_headers(&pair)).await else {
        panic!()
    };
    assert_eq!(a.access_token(), fresh);
    let next = a.set_cookie().expect("refreshed tokens are written back");
    let body = realm.bodies("/token").await.pop().unwrap();
    for part in [
        "grant_type=refresh_token",
        "refresh_token=rt-1",
        "client_secret=s3cret",
    ] {
        assert!(body.contains(part), "{part} in {body}");
    }
    // The rewritten cookie carries the new tokens: no second refresh.
    let next = next
        .to_str()
        .unwrap()
        .split(';')
        .next()
        .unwrap()
        .to_string();
    let Resolution::Active(b) = s.resolve(&cookie_headers(&next)).await else {
        panic!()
    };
    assert_eq!(b.access_token(), fresh);
    assert_eq!(
        realm.bodies("/token").await.len(),
        2,
        "sign-in + one refresh"
    );
}

#[tokio::test]
async fn a_refused_refresh_ends_the_session() {
    let realm = Realm::start().await;
    let s = HostSessions::new(config(&realm, &TestClock::new())).unwrap();
    let pair = sign_in(&s, &realm, 30).await;
    realm
        .on_refresh(ResponseTemplate::new(400).set_body_json(json!({"error": "invalid_grant"})))
        .await;
    assert!(matches!(
        s.resolve(&cookie_headers(&pair)).await,
        Resolution::Ended {
            reason: EndReason::RefreshRefused,
            clear_cookie: Some(_)
        }
    ));
}

#[tokio::test]
async fn an_unreachable_realm_keeps_the_session_until_the_token_expires() {
    let realm = Realm::start().await;
    let clock = TestClock::new();
    let s = HostSessions::new(config(&realm, &clock)).unwrap();
    let at_ttl = 30;
    let pair = sign_in(&s, &realm, at_ttl).await;
    realm.on_refresh(ResponseTemplate::new(503)).await;
    let Resolution::Active(a) = s.resolve(&cookie_headers(&pair)).await else {
        panic!("still-valid token is used while the realm is down")
    };
    assert_eq!(a.claims()["sub"], "user-1");
    clock.advance(at_ttl + 1);
    assert!(matches!(
        s.resolve(&cookie_headers(&pair)).await,
        Resolution::Ended {
            reason: EndReason::Unavailable,
            clear_cookie: None
        }
    ));
    let resp = call(&s, get("/_host/session", &[pair])).await;
    assert_eq!(resp.status(), StatusCode::SERVICE_UNAVAILABLE);
}

#[tokio::test]
async fn session_route_returns_display_claims_or_401() {
    let realm = Realm::start().await;
    let s = HostSessions::new(config(&realm, &TestClock::new())).unwrap();
    let resp = call(&s, get("/_host/session", &[])).await;
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    assert_eq!(resp.headers()[header::CACHE_CONTROL], "no-store");
    assert_eq!(body_json(resp).await, json!({"error": "unauthenticated"}));

    let pair = sign_in(&s, &realm, 300).await;
    let resp = call(&s, get("/_host/session", &[pair])).await;
    assert_eq!(resp.status(), StatusCode::OK);
    let body = body_json(resp).await;
    assert_eq!(body["sub"], "user-1");
    assert_eq!(body["name"], "Ana Souza");
    assert_eq!(body["organization"], json!({"clinic-a": {}}));
    assert!(body["exp"].as_i64().unwrap() > now_secs());
    assert!(body["idle_exp"].as_i64().unwrap() > now_secs());
    assert!(body.get("access_token").is_none());
}

#[tokio::test]
async fn logout_ends_the_realm_session_clears_the_cookie_and_redirects() {
    let realm = Realm::start().await;
    let s = HostSessions::new(config(&realm, &TestClock::new())).unwrap();
    let pair = sign_in(&s, &realm, 300).await;
    Mock::given(method("POST"))
        .and(path("/logout"))
        .respond_with(ResponseTemplate::new(204))
        .mount(&realm.server)
        .await;

    let cross = Request::post("/_host/logout")
        .header(header::COOKIE, &pair)
        .header(header::ORIGIN, "https://evil.example")
        .body(Body::empty())
        .unwrap();
    assert_eq!(call(&s, cross).await.status(), StatusCode::FORBIDDEN);
    assert!(realm.bodies("/logout").await.is_empty());

    assert_eq!(
        call(&s, get("/_host/logout", &[pair.clone()]))
            .await
            .status(),
        StatusCode::METHOD_NOT_ALLOWED
    );

    let req = Request::post("/_host/logout")
        .header(header::COOKIE, &pair)
        .header(header::ORIGIN, ORIGIN)
        .body(Body::empty())
        .unwrap();
    let resp = call(&s, req).await;
    assert_eq!(resp.status(), StatusCode::SEE_OTHER);
    let loc = location(&resp);
    assert!(loc.starts_with(&format!("{}/logout?", realm.server.uri())));
    let q = query(&loc);
    assert_eq!(q["client_id"], "meridis-apps-host");
    assert_eq!(q["post_logout_redirect_uri"], format!("{ORIGIN}/"));
    let cleared = set_cookies(&resp).join("\n");
    assert!(cleared.contains("__Host-session=; Path=/; Secure; HttpOnly; SameSite=Lax; Max-Age=0"));
    let body = realm
        .bodies("/logout")
        .await
        .pop()
        .expect("the realm session is ended server-side");
    for part in [
        "refresh_token=rt-1",
        "client_secret=s3cret",
        "client_id=meridis-apps-host",
    ] {
        assert!(body.contains(part), "{part} in {body}");
    }
}

/// Threat review 2 F3 (rudaia-hq/apps): a browser sends `Origin` on every POST,
/// so a logout POST with neither `Origin` nor `Sec-Fetch-Site` is refused. Either
/// one alone, from the host's own pages, still logs out.
#[tokio::test]
async fn logout_refuses_a_post_with_neither_origin_nor_sec_fetch_site() {
    let realm = Realm::start().await;
    let s = HostSessions::new(config(&realm, &TestClock::new())).unwrap();
    let pair = sign_in(&s, &realm, 300).await;
    Mock::given(method("POST"))
        .and(path("/logout"))
        .respond_with(ResponseTemplate::new(204))
        .mount(&realm.server)
        .await;

    let bare = Request::post("/_host/logout")
        .header(header::COOKIE, &pair)
        .body(Body::empty())
        .unwrap();
    let resp = call(&s, bare).await;
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);
    assert!(set_cookies(&resp).is_empty(), "the session stays");
    assert!(realm.bodies("/logout").await.is_empty());

    for (name, value) in [
        ("sec-fetch-site", "cross-site"),
        ("origin", "null"),
        ("origin", "https://evil.example"),
    ] {
        let req = Request::post("/_host/logout")
            .header(header::COOKIE, &pair)
            .header(name, value)
            .body(Body::empty())
            .unwrap();
        assert_eq!(
            call(&s, req).await.status(),
            StatusCode::FORBIDDEN,
            "{name}: {value}"
        );
    }
    assert!(realm.bodies("/logout").await.is_empty());

    for (name, value) in [("sec-fetch-site", "same-origin"), ("origin", ORIGIN)] {
        let req = Request::post("/_host/logout")
            .header(header::COOKIE, &pair)
            .header(name, value)
            .body(Body::empty())
            .unwrap();
        assert_eq!(
            call(&s, req).await.status(),
            StatusCode::SEE_OTHER,
            "{name}: {value}"
        );
    }
}

#[tokio::test]
async fn session_ttl_sets_max_age() {
    let realm = Realm::start().await;
    let mut cfg = config(&realm, &TestClock::new());
    cfg.session_ttl = Some(std::time::Duration::from_secs(900));
    let s = HostSessions::new(cfg).unwrap();
    let at = realm.access(300);
    let resp = sign_in_with(&s, &realm, |nonce| {
        json!({ "access_token": at, "refresh_token": "rt", "id_token": realm.id(nonce),
                "refresh_expires_in": 1800 })
    })
    .await;
    let c = set_cookies(&resp)
        .into_iter()
        .find(|c| c.starts_with("__Host-session="))
        .unwrap();
    assert!(c.ends_with("Max-Age=900"), "{c}");
}

#[tokio::test]
async fn cookie_size_is_checked_at_startup_and_at_sign_in() {
    let realm = Realm::start().await;
    let clock = TestClock::new();
    let mut cfg = config(&realm, &clock);
    cfg.expected_token_bytes = (6000, 1000);
    assert!(
        HostSessions::new(cfg).is_err(),
        "expected tokens would not fit"
    );

    let mut cfg = config(&realm, &clock);
    cfg.max_cookie_bytes = 600;
    cfg.expected_token_bytes = (300, 100);
    let s = HostSessions::new(cfg).unwrap();
    let at = realm
        .issuer
        .mint(json!({"sub": "user-1", "pad": "x".repeat(800)}));
    let resp = sign_in_with(&s, &realm, |nonce| {
        json!({ "access_token": at, "refresh_token": "rt", "id_token": realm.id(nonce),
                "refresh_expires_in": 1800 })
    })
    .await;
    assert_eq!(resp.status(), StatusCode::INTERNAL_SERVER_ERROR);
    assert!(cookie_pair(&resp, "__Host-session").is_none());
}

#[tokio::test]
async fn configuration_is_validated() {
    let realm = Realm::start().await;
    let clock = TestClock::new();
    let bad = |f: &dyn Fn(&mut HostSessionConfig)| {
        let mut cfg = config(&realm, &clock);
        f(&mut cfg);
        HostSessions::new(cfg).is_err()
    };
    assert!(bad(&|c| c.route_prefix = "/_host/".into()));
    assert!(bad(&|c| c.route_prefix = "_host".into()));
    assert!(bad(&|c| c.redirect_uri = format!("{ORIGIN}/callback")));
    assert!(bad(
        &|c| c.redirect_uri = "http://host.example/_host/callback".into()
    ));
    assert!(bad(&|c| c.binding = String::new()));
    assert!(bad(&|c| c.cookie_name = "bad name".into()));
    assert!(bad(&|c| c.algorithms.clear()));
    assert!(bad(&|c| c.issuer_url = "http://realm.example".into()));
    let s = HostSessions::new(config(&realm, &clock)).unwrap();
    assert_eq!(s.origin(), ORIGIN);
}
