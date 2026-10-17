//! Unit tests for auth feature: TokenProvider trait, AccessToken, AuthError, and
//! AuthenticatedHttpClient. Uses stub providers — no OIDC dependency.

use cli_framework::auth::{
    AccessToken, AuthError, AuthFlowReporter, AuthenticatedHttpClient, StderrAuthFlowReporter,
    TokenProvider, TokenStatus,
};
use cli_framework::http_retry::RetryableHttpClient;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::SystemTime;
use wiremock::matchers::{header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

// ── Stub providers ────────────────────────────────────────────────────────────

/// Always returns a fresh token; never requires login.
struct AlwaysOkProvider {
    token: String,
    expires_at: Option<SystemTime>,
    invalidate_count: Arc<AtomicUsize>,
}

#[async_trait::async_trait]
impl TokenProvider for AlwaysOkProvider {
    async fn token(&self) -> Result<AccessToken, AuthError> {
        Ok(AccessToken::new(self.token.clone(), self.expires_at))
    }
    async fn invalidate(&self) {
        self.invalidate_count.fetch_add(1, Ordering::SeqCst);
    }
    async fn peek(&self) -> Option<TokenStatus> {
        Some(TokenStatus {
            logged_in: true,
            expires_at: self.expires_at,
        })
    }
}

// ── TokenProvider trait defaults ──────────────────────────────────────────────

#[tokio::test]
async fn token_provider_default_peek_returns_none() {
    struct MinimalProvider;
    #[async_trait::async_trait]
    impl TokenProvider for MinimalProvider {
        async fn token(&self) -> Result<AccessToken, AuthError> {
            Err(AuthError::NotAuthenticated)
        }
        async fn invalidate(&self) {}
    }

    let p = MinimalProvider;
    assert!(p.peek().await.is_none(), "default peek() must return None");
}

#[tokio::test]
async fn token_provider_default_login_returns_not_supported() {
    struct MinimalProvider;
    #[async_trait::async_trait]
    impl TokenProvider for MinimalProvider {
        async fn token(&self) -> Result<AccessToken, AuthError> {
            Err(AuthError::NotAuthenticated)
        }
        async fn invalidate(&self) {}
    }

    let p = MinimalProvider;
    let err = p.login().await.unwrap_err();
    assert!(
        matches!(err, AuthError::NotSupported("login")),
        "default login() must return NotSupported(\"login\")"
    );
}

#[tokio::test]
async fn token_provider_default_logout_returns_not_supported() {
    struct MinimalProvider;
    #[async_trait::async_trait]
    impl TokenProvider for MinimalProvider {
        async fn token(&self) -> Result<AccessToken, AuthError> {
            Err(AuthError::NotAuthenticated)
        }
        async fn invalidate(&self) {}
    }

    let p = MinimalProvider;
    let err = p.logout().await.unwrap_err();
    assert!(
        matches!(err, AuthError::NotSupported("logout")),
        "default logout() must return NotSupported(\"logout\")"
    );
}

// ── AccessToken Debug redaction ───────────────────────────────────────────────

#[test]
fn access_token_debug_redacts_raw() {
    let tok = AccessToken::new("super-secret-bearer-value".to_string(), None);
    let debug_str = format!("{:?}", tok);
    assert!(
        !debug_str.contains("super-secret-bearer-value"),
        "AccessToken Debug must not print the raw token; got: {debug_str}"
    );
    assert!(
        debug_str.contains("***"),
        "AccessToken Debug must contain '***'; got: {debug_str}"
    );
}

#[test]
fn access_token_as_bearer_returns_raw() {
    let tok = AccessToken::new("my-token".to_string(), None);
    assert_eq!(tok.as_bearer(), "my-token");
}

// ── AuthenticatedHttpClient: bearer injection ─────────────────────────────────

#[tokio::test]
async fn authenticated_client_injects_bearer_header() {
    let mock = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/resource"))
        .and(header("authorization", "Bearer my-token"))
        .respond_with(ResponseTemplate::new(200))
        .expect(1)
        .mount(&mock)
        .await;

    let provider = Arc::new(AlwaysOkProvider {
        token: "my-token".to_string(),
        expires_at: None,
        invalidate_count: Arc::new(AtomicUsize::new(0)),
    });

    let inner = RetryableHttpClient::new(reqwest::Client::new());
    let client = AuthenticatedHttpClient::new(inner, provider);

    let url = format!("{}/api/resource", mock.uri());
    let resp = client.get(&url).await.expect("request should succeed");
    assert_eq!(resp.status(), 200);

    mock.verify().await;
}

// ── AuthenticatedHttpClient: 401 → invalidate + retry ────────────────────────

#[tokio::test]
async fn authenticated_client_retries_once_on_401() {
    let mock = MockServer::start().await;

    // First call: 401. Second call (with refreshed token): 200.
    Mock::given(method("GET"))
        .and(path("/api/data"))
        .and(header("authorization", "Bearer token-v1"))
        .respond_with(ResponseTemplate::new(401))
        .expect(1)
        .named("first-401")
        .mount(&mock)
        .await;

    Mock::given(method("GET"))
        .and(path("/api/data"))
        .and(header("authorization", "Bearer token-v2"))
        .respond_with(ResponseTemplate::new(200))
        .expect(1)
        .named("second-200")
        .mount(&mock)
        .await;

    let call_count = Arc::new(AtomicUsize::new(0));
    let invalidate_count = Arc::new(AtomicUsize::new(0));
    let cc = call_count.clone();
    let ic = invalidate_count.clone();

    struct TwoTokenProvider {
        call_count: Arc<AtomicUsize>,
        invalidate_count: Arc<AtomicUsize>,
    }
    #[async_trait::async_trait]
    impl TokenProvider for TwoTokenProvider {
        async fn token(&self) -> Result<AccessToken, AuthError> {
            let n = self.call_count.fetch_add(1, Ordering::SeqCst);
            let tok = if n == 0 { "token-v1" } else { "token-v2" };
            Ok(AccessToken::new(tok.to_string(), None))
        }
        async fn invalidate(&self) {
            self.invalidate_count.fetch_add(1, Ordering::SeqCst);
        }
    }

    let provider = Arc::new(TwoTokenProvider {
        call_count: cc,
        invalidate_count: ic,
    });
    let inner = RetryableHttpClient::new(reqwest::Client::new());
    let auth_client = AuthenticatedHttpClient::new(inner, provider);

    let url = format!("{}/api/data", mock.uri());
    let resp = auth_client
        .get(&url)
        .await
        .expect("should succeed on retry");
    assert_eq!(resp.status(), 200);
    assert_eq!(
        invalidate_count.load(Ordering::SeqCst),
        1,
        "invalidate called once"
    );

    mock.verify().await;
}

// ── AuthenticatedHttpClient: double 401 → surfaces error ─────────────────────

#[tokio::test]
async fn authenticated_client_surfaces_error_on_second_401() {
    let mock = MockServer::start().await;
    // Both calls return 401 — never recovers.
    Mock::given(method("GET"))
        .and(path("/api/secure"))
        .respond_with(ResponseTemplate::new(401))
        .expect(2)
        .mount(&mock)
        .await;

    let provider = Arc::new(AlwaysOkProvider {
        token: "stale-token".to_string(),
        expires_at: None,
        invalidate_count: Arc::new(AtomicUsize::new(0)),
    });

    let inner = RetryableHttpClient::new(reqwest::Client::new());
    let client = AuthenticatedHttpClient::new(inner, provider);

    let url = format!("{}/api/secure", mock.uri());
    let err = client.get(&url).await.unwrap_err();
    // Error should be present; we just verify no infinite loop occurred.
    let _ = err;

    mock.verify().await;
}

// ── AuthenticatedHttpClient: NotAuthenticated on re-fetch → downcasts ────────

#[tokio::test]
async fn authenticated_client_not_authenticated_surfaces_as_auth_error() {
    let mock = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/x"))
        .respond_with(ResponseTemplate::new(401))
        .expect(1)
        .mount(&mock)
        .await;

    // First token() succeeds; after invalidate, NotAuthenticated.
    let call_count = Arc::new(AtomicUsize::new(0));
    struct FirstThenNotAuth(Arc<AtomicUsize>);
    #[async_trait::async_trait]
    impl TokenProvider for FirstThenNotAuth {
        async fn token(&self) -> Result<AccessToken, AuthError> {
            let n = self.0.fetch_add(1, Ordering::SeqCst);
            if n == 0 {
                Ok(AccessToken::new("tk".to_string(), None))
            } else {
                Err(AuthError::NotAuthenticated)
            }
        }
        async fn invalidate(&self) {}
    }

    let provider = Arc::new(FirstThenNotAuth(call_count));
    let inner = RetryableHttpClient::new(reqwest::Client::new());
    let client = AuthenticatedHttpClient::new(inner, provider);

    let url = format!("{}/api/x", mock.uri());
    let err = client.get(&url).await.unwrap_err();
    assert!(
        matches!(
            err.downcast_ref::<AuthError>(),
            Some(AuthError::NotAuthenticated)
        ),
        "error must downcast to AuthError::NotAuthenticated; got: {err:?}"
    );

    mock.verify().await;
}

// ── AuthFlowReporter ──────────────────────────────────────────────────────────

#[test]
fn stderr_auth_flow_reporter_compiles() {
    // Just ensure StderrAuthFlowReporter is constructible and usable as trait object.
    let reporter: Arc<dyn AuthFlowReporter> = Arc::new(StderrAuthFlowReporter);
    // Calling these writes to real stderr; we just assert they don't panic.
    reporter.message("test message");
}

// ── AuthenticatedHttpClient::send: one attempt, any status ───────────────────

/// Hands out `token-v1`, then `token-v2` after `invalidate`, counting both.
struct RotatingProvider {
    calls: AtomicUsize,
    invalidated: AtomicUsize,
}

impl RotatingProvider {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            calls: AtomicUsize::new(0),
            invalidated: AtomicUsize::new(0),
        })
    }
}

#[async_trait::async_trait]
impl TokenProvider for RotatingProvider {
    async fn token(&self) -> Result<AccessToken, AuthError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        let token = match self.invalidated.load(Ordering::SeqCst) {
            0 => "token-v1",
            _ => "token-v2",
        };
        Ok(AccessToken::new(token.to_string(), None))
    }
    async fn invalidate(&self) {
        self.invalidated.fetch_add(1, Ordering::SeqCst);
    }
}

fn send_client(provider: Arc<dyn TokenProvider>) -> AuthenticatedHttpClient {
    AuthenticatedHttpClient::new(RetryableHttpClient::new(reqwest::Client::new()), provider)
}

#[tokio::test]
async fn send_returns_an_error_answer_with_its_body_and_does_not_retry_it() {
    for status in [400u16, 403, 409, 429, 500, 503] {
        let mock = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/api/write"))
            .and(header("authorization", "Bearer token-v1"))
            .respond_with(ResponseTemplate::new(status).set_body_string(r#"{"code":"no"}"#))
            .expect(1)
            .mount(&mock)
            .await;
        let provider = RotatingProvider::new();
        let client = send_client(provider.clone());

        let url = format!("{}/api/write", mock.uri());
        let response = client
            .send(|| client.client().post(&url).body("once"))
            .await
            .expect("an error status is an answer, not an error");
        assert_eq!(response.status(), status);
        assert_eq!(response.text().await.unwrap(), r#"{"code":"no"}"#);
        assert_eq!(provider.invalidated.load(Ordering::SeqCst), 0, "{status}");
        mock.verify().await;
    }
}

#[tokio::test]
async fn send_renews_the_token_once_on_401_and_returns_the_second_answer() {
    let mock = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/write"))
        .and(header("authorization", "Bearer token-v1"))
        .respond_with(ResponseTemplate::new(401))
        .expect(1)
        .mount(&mock)
        .await;
    Mock::given(method("POST"))
        .and(path("/api/write"))
        .and(header("authorization", "Bearer token-v2"))
        .respond_with(ResponseTemplate::new(201).set_body_string("made"))
        .expect(1)
        .mount(&mock)
        .await;
    let provider = RotatingProvider::new();
    let client = send_client(provider.clone());

    let url = format!("{}/api/write", mock.uri());
    let response = client.send(|| client.client().post(&url)).await.unwrap();
    assert_eq!(response.status(), 201);
    assert_eq!(response.text().await.unwrap(), "made");
    assert_eq!(provider.invalidated.load(Ordering::SeqCst), 1);
    assert_eq!(provider.calls.load(Ordering::SeqCst), 2);
    mock.verify().await;
}

#[tokio::test]
async fn send_returns_a_second_401_as_an_answer() {
    let mock = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/secure"))
        .respond_with(ResponseTemplate::new(401).set_body_string("who"))
        .expect(2)
        .mount(&mock)
        .await;
    let provider = RotatingProvider::new();
    let client = send_client(provider.clone());

    let url = format!("{}/api/secure", mock.uri());
    let response = client.send(|| client.client().get(&url)).await.unwrap();
    assert_eq!(response.status(), 401);
    assert_eq!(response.text().await.unwrap(), "who");
    assert_eq!(provider.invalidated.load(Ordering::SeqCst), 1);
    mock.verify().await;
}

#[tokio::test]
async fn send_surfaces_not_authenticated_and_sends_nothing_without_a_token() {
    struct NoLogin;
    #[async_trait::async_trait]
    impl TokenProvider for NoLogin {
        async fn token(&self) -> Result<AccessToken, AuthError> {
            Err(AuthError::NotAuthenticated)
        }
        async fn invalidate(&self) {}
    }
    let mock = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(200))
        .expect(0)
        .mount(&mock)
        .await;
    let client = send_client(Arc::new(NoLogin));

    let url = format!("{}/api/x", mock.uri());
    let err = client.send(|| client.client().get(&url)).await.unwrap_err();
    assert!(
        matches!(
            err.downcast_ref::<AuthError>(),
            Some(AuthError::NotAuthenticated)
        ),
        "{err:?}"
    );
    mock.verify().await;
}

#[tokio::test]
async fn send_does_not_retry_a_request_that_got_no_answer() {
    // A port nothing listens on: bind, note it, close.
    let port = std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    let provider = RotatingProvider::new();
    let client = send_client(provider.clone());

    let url = format!("http://127.0.0.1:{port}/api/write");
    let err = client
        .send(|| client.client().post(&url))
        .await
        .unwrap_err();
    assert!(
        err.downcast_ref::<reqwest::Error>()
            .is_some_and(reqwest::Error::is_connect),
        "{err:?}"
    );
    assert_eq!(provider.calls.load(Ordering::SeqCst), 1, "sent once");
}
