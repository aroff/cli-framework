//! Integration tests for the `auth` built-in commands via `CliTestHarness`.
//!
//! Asserts exit codes, stdout, and stderr per the spec 015 stream contract.

use cli_framework::app::{AppBuilder, AppContext};
use cli_framework::auth::{AccessToken, AuthError, TokenProvider, TokenStatus};
use cli_framework::testkit::CliTestHarness;
use std::sync::Arc;
use std::time::{Duration, SystemTime};

struct Ctx;
impl AppContext for Ctx {}

// ── Stub providers ────────────────────────────────────────────────────────────

struct LoggedInProvider {
    expires_at: Option<SystemTime>,
}
#[async_trait::async_trait]
impl TokenProvider for LoggedInProvider {
    async fn token(&self) -> Result<AccessToken, AuthError> {
        Ok(AccessToken::new("bearer-123".to_string(), self.expires_at))
    }
    async fn invalidate(&self) {}
    async fn peek(&self) -> Option<TokenStatus> {
        Some(TokenStatus {
            logged_in: true,
            expires_at: self.expires_at,
        })
    }
    async fn login(&self) -> Result<(), AuthError> {
        Ok(())
    }
    async fn logout(&self) -> Result<(), AuthError> {
        Ok(())
    }
}

struct NotAuthProvider;
#[async_trait::async_trait]
impl TokenProvider for NotAuthProvider {
    async fn token(&self) -> Result<AccessToken, AuthError> {
        Err(AuthError::NotAuthenticated)
    }
    async fn invalidate(&self) {}
    async fn peek(&self) -> Option<TokenStatus> {
        Some(TokenStatus {
            logged_in: false,
            expires_at: None,
        })
    }
    async fn login(&self) -> Result<(), AuthError> {
        Ok(())
    }
    async fn logout(&self) -> Result<(), AuthError> {
        Ok(())
    }
}

struct NotSupportedProvider;
#[async_trait::async_trait]
impl TokenProvider for NotSupportedProvider {
    async fn token(&self) -> Result<AccessToken, AuthError> {
        Err(AuthError::NotAuthenticated)
    }
    async fn invalidate(&self) {}
    // login/logout use the default NotSupported impl
}

// ── auth token ────────────────────────────────────────────────────────────────

#[tokio::test]
async fn auth_token_logged_in_prints_bearer_to_stdout_exit_0() {
    let app = AppBuilder::new()
        .with_version("myapp", "1.0")
        .with_token_provider(Arc::new(LoggedInProvider { expires_at: None }))
        .build(Ctx)
        .unwrap();
    let mut h = CliTestHarness::new(app);
    let out = h.run(&["myapp", "auth", "token"]).await;

    assert_eq!(out.exit_code(), 0, "logged-in auth token must exit 0");
    assert_eq!(
        out.stdout().trim(),
        "bearer-123",
        "stdout must be the raw bearer token"
    );
    assert!(out.stderr().is_empty(), "no stderr on success");
}

#[tokio::test]
async fn auth_token_not_authenticated_exits_1_with_auth003() {
    let app = AppBuilder::new()
        .with_version("myapp", "1.0")
        .with_token_provider(Arc::new(NotAuthProvider))
        .build(Ctx)
        .unwrap();
    let mut h = CliTestHarness::new(app);
    let out = h.run(&["myapp", "auth", "token"]).await;

    assert_eq!(out.exit_code(), 1, "NotAuthenticated must exit 1");
    assert!(
        out.stdout().is_empty(),
        "stdout must be empty on NotAuthenticated"
    );
    out.assert_diagnostic_code("AUTH003");
}

// ── auth status ───────────────────────────────────────────────────────────────

#[tokio::test]
async fn auth_status_logged_out_stdout_exit_0() {
    let app = AppBuilder::new()
        .with_version("myapp", "1.0")
        .with_token_provider(Arc::new(NotAuthProvider))
        .build(Ctx)
        .unwrap();
    let mut h = CliTestHarness::new(app);
    let out = h.run(&["myapp", "auth", "status"]).await;

    assert_eq!(out.exit_code(), 0, "logged-out status must exit 0 (query)");
    assert!(
        out.stdout().to_lowercase().contains("not logged in"),
        "stdout must say not logged in; got: {:?}",
        out.stdout()
    );
}

#[tokio::test]
async fn auth_status_logged_in_exit_0() {
    let app = AppBuilder::new()
        .with_version("myapp", "1.0")
        .with_token_provider(Arc::new(LoggedInProvider { expires_at: None }))
        .build(Ctx)
        .unwrap();
    let mut h = CliTestHarness::new(app);
    let out = h.run(&["myapp", "auth", "status"]).await;

    assert_eq!(out.exit_code(), 0);
    assert!(
        out.stdout().to_lowercase().contains("logged in"),
        "stdout must say logged in; got: {:?}",
        out.stdout()
    );
}

#[tokio::test]
async fn auth_status_json_logged_in_exit_0() {
    let expires_at = SystemTime::now() + Duration::from_secs(3600);
    let app = AppBuilder::new()
        .with_version("myapp", "1.0")
        .with_token_provider(Arc::new(LoggedInProvider {
            expires_at: Some(expires_at),
        }))
        .build(Ctx)
        .unwrap();
    let mut h = CliTestHarness::new(app);
    let out = h.run(&["myapp", "auth", "status", "--json"]).await;

    assert_eq!(out.exit_code(), 0);
    let v: serde_json::Value =
        serde_json::from_str(out.stdout()).expect("--json must produce valid JSON");
    assert_eq!(v["logged_in"], serde_json::Value::Bool(true));
    assert!(
        v.get("expires_at").is_some(),
        "expires_at key must be present"
    );
    assert!(
        v.get("expires_in_seconds").is_some(),
        "expires_in_seconds key must be present"
    );
}

#[tokio::test]
async fn auth_status_json_logged_out_exit_0() {
    let app = AppBuilder::new()
        .with_version("myapp", "1.0")
        .with_token_provider(Arc::new(NotAuthProvider))
        .build(Ctx)
        .unwrap();
    let mut h = CliTestHarness::new(app);
    let out = h.run(&["myapp", "auth", "status", "--json"]).await;

    assert_eq!(
        out.exit_code(),
        0,
        "--json must exit 0 even when logged out"
    );
    let v: serde_json::Value =
        serde_json::from_str(out.stdout()).expect("--json must produce valid JSON");
    assert_eq!(v["logged_in"], serde_json::Value::Bool(false));
}

#[tokio::test]
async fn auth_status_no_refresh_with_peek_support() {
    let app = AppBuilder::new()
        .with_version("myapp", "1.0")
        .with_token_provider(Arc::new(LoggedInProvider { expires_at: None }))
        .build(Ctx)
        .unwrap();
    let mut h = CliTestHarness::new(app);
    let out = h.run(&["myapp", "auth", "status", "--no-refresh"]).await;

    assert_eq!(out.exit_code(), 0);
}

#[tokio::test]
async fn auth_status_no_refresh_no_peek_emits_plain_stderr_exit_0() {
    // NotSupportedProvider.peek() returns None.
    let app = AppBuilder::new()
        .with_version("myapp", "1.0")
        .with_token_provider(Arc::new(NotSupportedProvider))
        .build(Ctx)
        .unwrap();
    let mut h = CliTestHarness::new(app);
    let out = h.run(&["myapp", "auth", "status", "--no-refresh"]).await;

    assert_eq!(out.exit_code(), 0, "peek=None must still exit 0");
    assert!(
        out.stderr()
            .contains("status unavailable in read-only mode"),
        "must print pinned message on stderr; got: {:?}",
        out.stderr()
    );
}

// ── A3: --no-refresh with peek()=Some prints status ──────────────────────────

#[tokio::test]
async fn auth_status_no_refresh_with_peek_logged_in_prints_logged_in() {
    let app = AppBuilder::new()
        .with_version("myapp", "1.0")
        .with_token_provider(Arc::new(LoggedInProvider { expires_at: None }))
        .build(Ctx)
        .unwrap();
    let mut h = CliTestHarness::new(app);
    let out = h.run(&["myapp", "auth", "status", "--no-refresh"]).await;

    assert_eq!(out.exit_code(), 0);
    assert!(
        out.stdout().to_lowercase().contains("logged in"),
        "--no-refresh with peek()=Some(logged_in=true) must print status; got stdout: {:?}",
        out.stdout()
    );
}

#[tokio::test]
async fn auth_status_no_refresh_with_peek_logged_out_prints_not_logged_in() {
    let app = AppBuilder::new()
        .with_version("myapp", "1.0")
        .with_token_provider(Arc::new(NotAuthProvider))
        .build(Ctx)
        .unwrap();
    let mut h = CliTestHarness::new(app);
    let out = h.run(&["myapp", "auth", "status", "--no-refresh"]).await;

    assert_eq!(out.exit_code(), 0);
    assert!(
        out.stdout().to_lowercase().contains("not logged in"),
        "--no-refresh with peek()=Some(logged_in=false) must print 'not logged in'; got: {:?}",
        out.stdout()
    );
}

// ── A2: expiry-unknown text ───────────────────────────────────────────────────

#[tokio::test]
async fn auth_status_logged_in_no_expiry_says_expiry_unknown() {
    let app = AppBuilder::new()
        .with_version("myapp", "1.0")
        .with_token_provider(Arc::new(LoggedInProvider { expires_at: None }))
        .build(Ctx)
        .unwrap();
    let mut h = CliTestHarness::new(app);
    let out = h.run(&["myapp", "auth", "status"]).await;

    assert_eq!(out.exit_code(), 0);
    assert!(
        out.stdout().contains("expiry unknown"),
        "stdout must contain 'expiry unknown' when expires_at is None; got: {:?}",
        out.stdout()
    );
}

// ── auth login ────────────────────────────────────────────────────────────────

#[tokio::test]
async fn auth_login_success_exit_0_stderr_confirmation() {
    let app = AppBuilder::new()
        .with_version("myapp", "1.0")
        .with_token_provider(Arc::new(LoggedInProvider { expires_at: None }))
        .build(Ctx)
        .unwrap();
    let mut h = CliTestHarness::new(app);
    let out = h.run(&["myapp", "auth", "login"]).await;

    assert_eq!(out.exit_code(), 0);
    assert!(
        out.stderr().contains("Logged in"),
        "success must print 'Logged in' on stderr; got: {:?}",
        out.stderr()
    );
}

#[tokio::test]
async fn auth_login_not_supported_exit_1_auth001() {
    let app = AppBuilder::new()
        .with_version("myapp", "1.0")
        .with_token_provider(Arc::new(NotSupportedProvider))
        .build(Ctx)
        .unwrap();
    let mut h = CliTestHarness::new(app);
    let out = h.run(&["myapp", "auth", "login"]).await;

    assert_eq!(out.exit_code(), 1, "NotSupported login must exit 1");
    out.assert_diagnostic_code("AUTH001");
}

// ── auth logout ───────────────────────────────────────────────────────────────

#[tokio::test]
async fn auth_logout_success_exit_0() {
    let app = AppBuilder::new()
        .with_version("myapp", "1.0")
        .with_token_provider(Arc::new(LoggedInProvider { expires_at: None }))
        .build(Ctx)
        .unwrap();
    let mut h = CliTestHarness::new(app);
    let out = h.run(&["myapp", "auth", "logout"]).await;
    assert_eq!(out.exit_code(), 0);
}

#[tokio::test]
async fn auth_logout_not_supported_exit_1_auth001() {
    let app = AppBuilder::new()
        .with_version("myapp", "1.0")
        .with_token_provider(Arc::new(NotSupportedProvider))
        .build(Ctx)
        .unwrap();
    let mut h = CliTestHarness::new(app);
    let out = h.run(&["myapp", "auth", "logout"]).await;

    assert_eq!(out.exit_code(), 1);
    out.assert_diagnostic_code("AUTH001");
}

// ── auth commands never trigger an interactive flow ───────────────────────────

#[tokio::test]
async fn auth_status_and_token_never_call_login() {
    // A provider that panics if login() is called.
    struct PanicOnLoginProvider;
    #[async_trait::async_trait]
    impl TokenProvider for PanicOnLoginProvider {
        async fn token(&self) -> Result<AccessToken, AuthError> {
            Err(AuthError::NotAuthenticated)
        }
        async fn invalidate(&self) {}
        async fn login(&self) -> Result<(), AuthError> {
            panic!("login() was called — auth status/token must not trigger an interactive flow");
        }
    }

    let provider = Arc::new(PanicOnLoginProvider);

    for cmd in &["status", "token"] {
        let app = AppBuilder::new()
            .with_version("myapp", "1.0")
            .with_token_provider(provider.clone())
            .build(Ctx)
            .unwrap();
        let mut h = CliTestHarness::new(app);
        // Must not panic (login() must not be called).
        let _ = h.run(&["myapp", "auth", cmd]).await;
    }
}
