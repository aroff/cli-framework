use cli_framework::auth::{AccessToken, AuthError, TokenProvider, TokenStatus};
use cli_framework::prelude::*;
use std::sync::Arc;
use std::time::{Duration, SystemTime};

struct AppCtx;
impl AppContext for AppCtx {}

/// A stub provider that reads a bearer token from the `MY_APP_TOKEN` env variable.
/// In a real application, replace this with `cli_framework_oidc::OidcClient`.
struct EnvTokenProvider;

#[async_trait::async_trait]
impl TokenProvider for EnvTokenProvider {
    async fn token(&self) -> Result<AccessToken, AuthError> {
        match std::env::var("MY_APP_TOKEN") {
            Ok(t) if !t.is_empty() => Ok(AccessToken::new(
                t,
                Some(SystemTime::now() + Duration::from_secs(3600)),
            )),
            _ => Err(AuthError::NotAuthenticated),
        }
    }

    async fn invalidate(&self) {
        // No local state to clear for env-var tokens.
    }

    async fn peek(&self) -> Option<TokenStatus> {
        let logged_in = std::env::var("MY_APP_TOKEN")
            .map(|v| !v.is_empty())
            .unwrap_or(false);
        Some(TokenStatus {
            logged_in,
            expires_at: None,
        })
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let mut app = AppBuilder::new()
        .with_version("with-auth", "0.1.0")
        .with_token_provider(Arc::new(EnvTokenProvider))
        .build(AppCtx)?;

    app.run().await
}
