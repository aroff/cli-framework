//! Authentication abstractions for cli-framework.
//!
//! Provides a [`TokenProvider`] trait and [`AuthenticatedHttpClient`] wrapper
//! that automatically injects bearer tokens and handles 401 refresh.
//!
//! Enable with `features = ["auth"]`.

use async_trait::async_trait;
use std::sync::Arc;
use std::time::SystemTime;

pub mod commands;

// ── TokenStatus ───────────────────────────────────────────────────────────────

/// Read-only snapshot of authentication state.
#[derive(Clone, Debug)]
pub struct TokenStatus {
    pub logged_in: bool,
    pub expires_at: Option<SystemTime>,
}

// ── AccessToken ──────────────────────────────────────────────────────────────

/// Opaque bearer token with optional expiry.
///
/// The raw token value is intentionally excluded from `Debug` output to
/// prevent accidental credential logging.
#[derive(Clone)]
pub struct AccessToken {
    raw: String,
    expires_at: Option<SystemTime>,
}

impl AccessToken {
    pub fn new(raw: String, expires_at: Option<SystemTime>) -> Self {
        Self { raw, expires_at }
    }

    pub fn as_bearer(&self) -> &str {
        &self.raw
    }

    pub fn expires_at(&self) -> Option<SystemTime> {
        self.expires_at
    }
}

impl std::fmt::Debug for AccessToken {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AccessToken")
            .field("raw", &"***")
            .field("expires_at", &self.expires_at)
            .finish()
    }
}

// ── AuthError ────────────────────────────────────────────────────────────────

#[derive(thiserror::Error, Debug)]
#[non_exhaustive]
pub enum AuthError {
    #[error("not authenticated; run `auth login`")]
    NotAuthenticated,

    #[error("operation not supported by this provider: {0}")]
    NotSupported(&'static str),

    #[error("authentication provider error: {message}")]
    Provider {
        message: String,
        #[source]
        source: Option<Box<dyn std::error::Error + Send + Sync>>,
    },
}

// ── TokenProvider ─────────────────────────────────────────────────────────────

#[async_trait]
pub trait TokenProvider: Send + Sync + 'static {
    /// Acquire (or return a cached) access token.
    async fn token(&self) -> Result<AccessToken, AuthError>;

    /// Invalidate any cached token so the next call to `token()` re-acquires one.
    async fn invalidate(&self);

    /// Return a read-only status snapshot without triggering a network refresh.
    /// Returns `None` if the provider does not support peek.
    async fn peek(&self) -> Option<TokenStatus> {
        None
    }

    /// Perform an interactive login flow and cache the resulting credentials.
    async fn login(&self) -> Result<(), AuthError> {
        Err(AuthError::NotSupported("login"))
    }

    /// Clear cached credentials.
    async fn logout(&self) -> Result<(), AuthError> {
        Err(AuthError::NotSupported("logout"))
    }
}

// ── AuthFlowReporter ─────────────────────────────────────────────────────────

/// Sink for user-visible messages produced during an interactive auth flow.
pub trait AuthFlowReporter: Send + Sync {
    /// Present a device-flow user code to the operator.
    fn user_code(&self, verification_uri: &str, user_code: &str);
    /// Emit a generic informational line.
    fn message(&self, line: &str);
}

/// Default [`AuthFlowReporter`] — writes directly to stderr.
pub struct StderrAuthFlowReporter;

impl AuthFlowReporter for StderrAuthFlowReporter {
    fn user_code(&self, verification_uri: &str, user_code: &str) {
        use std::io::Write;
        let _ = writeln!(
            std::io::stderr(),
            "Open {verification_uri} and enter code: {user_code}"
        );
    }

    fn message(&self, line: &str) {
        use std::io::Write;
        let _ = writeln!(std::io::stderr(), "{line}");
    }
}

// ── AuthenticatedHttpClient ───────────────────────────────────────────────────

/// Wraps [`crate::http_retry::RetryableHttpClient`] and injects a bearer token
/// from a [`TokenProvider`] on every request.
///
/// On 401 responses the token is invalidated, re-acquired once, and the
/// request is retried a single time before surfacing the error.
pub struct AuthenticatedHttpClient {
    inner: crate::http_retry::RetryableHttpClient,
    provider: Arc<dyn TokenProvider>,
}

impl AuthenticatedHttpClient {
    pub fn new(
        inner: crate::http_retry::RetryableHttpClient,
        provider: Arc<dyn TokenProvider>,
    ) -> Self {
        Self { inner, provider }
    }

    /// Access the underlying `reqwest::Client`.
    pub fn client(&self) -> &reqwest::Client {
        self.inner.client()
    }

    /// Execute an arbitrary request closure, injecting the bearer token and
    /// retrying once on 401.
    pub async fn execute_with_retry<F>(&self, build: F) -> anyhow::Result<reqwest::Response>
    where
        F: Fn() -> reqwest::RequestBuilder + Send + Sync,
    {
        let token = self.provider.token().await.map_err(anyhow::Error::from)?;
        let bearer = token.as_bearer().to_string();

        let result = self
            .inner
            .execute_with_retry(|| build().bearer_auth(&bearer))
            .await;

        match result {
            Ok(resp) => Ok(resp),
            Err(ref e) if is_unauthorized(e) => {
                // Invalidate and try once more.
                self.provider.invalidate().await;
                let fresh = self.provider.token().await.map_err(anyhow::Error::from)?;
                let fresh_bearer = fresh.as_bearer().to_string();
                self.inner
                    .execute_with_retry(|| build().bearer_auth(&fresh_bearer))
                    .await
            }
            Err(e) => Err(e),
        }
    }

    pub async fn get(&self, url: &str) -> anyhow::Result<reqwest::Response> {
        self.execute_with_retry(|| self.inner.client().get(url))
            .await
    }

    pub async fn post(&self, url: &str) -> anyhow::Result<reqwest::Response> {
        self.execute_with_retry(|| self.inner.client().post(url))
            .await
    }

    pub async fn put(&self, url: &str) -> anyhow::Result<reqwest::Response> {
        self.execute_with_retry(|| self.inner.client().put(url))
            .await
    }

    pub async fn delete(&self, url: &str) -> anyhow::Result<reqwest::Response> {
        self.execute_with_retry(|| self.inner.client().delete(url))
            .await
    }

    pub async fn patch(&self, url: &str) -> anyhow::Result<reqwest::Response> {
        self.execute_with_retry(|| self.inner.client().patch(url))
            .await
    }

    pub async fn head(&self, url: &str) -> anyhow::Result<reqwest::Response> {
        self.execute_with_retry(|| self.inner.client().head(url))
            .await
    }

    pub async fn options(&self, url: &str) -> anyhow::Result<reqwest::Response> {
        use reqwest::Method;
        self.execute_with_retry(|| self.inner.client().request(Method::OPTIONS, url))
            .await
    }
}

/// Returns `true` when the error wraps a 401 Unauthorized from reqwest.
fn is_unauthorized(e: &anyhow::Error) -> bool {
    e.downcast_ref::<reqwest::Error>()
        .and_then(|re| re.status())
        == Some(reqwest::StatusCode::UNAUTHORIZED)
}
