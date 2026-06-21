/// Browser OIDC authentication for SPA products.
///
/// Provides two tower Layers:
/// - `oidc_browser_session_layer`: for HTML/UI routes — validates session cookie,
///   redirects to Keycloak on miss, handles /callback and /logout.
/// - `oidc_dual_mode_layer`: for /api/* routes — accepts Bearer JWT (Agents)
///   or Session Cookie (browser fetch). Bearer takes precedence.
pub mod auth_state;
pub mod cookie;
pub(crate) mod dual;
pub(crate) mod handlers;
pub(crate) mod layer;
pub mod pkce;
pub mod request_type;
pub mod session_key;
pub(crate) mod state;

pub use crate::types::{AudiencePolicy, OidcClaims};
pub use session_key::SessionKey;

use crate::jwks::JwksCache;
use crate::OidcConfigError;
use jsonwebtoken::Algorithm;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{Mutex, OnceCell};

use auth_state::derive_hmac_key;
use cookie::estimate_cookie_size;
use state::BrowserLayerState;

/// Maximum access token size used in the startup-time cookie size check.
const DEFAULT_MAX_ACCESS_TOKEN_SIZE: usize = 2048;

/// Configuration for the browser session layer.
#[derive(Clone)]
pub struct OidcBrowserSessionConfig {
    /// OIDC issuer URL (normalized via normalize_issuer).
    pub issuer_url: String,
    /// Keycloak public client_id (PKCE only, no client_secret).
    pub client_id: String,
    /// Full callback URL registered in Keycloak.
    pub redirect_uri: String,
    /// 32-byte AES-256-GCM session key (provisioned via OpenBao).
    pub session_key: SessionKey,
    /// Route path for the callback handler (default "/callback").
    pub callback_path: String,
    /// Cookie name (default "session").
    pub cookie_name: String,
    /// Maximum session duration (default 8h). Cookie Max-Age = min(refresh_token_exp - now, session_ttl).
    pub session_ttl: Duration,
    /// How far before access token exp to proactively refresh (default 60s).
    pub refresh_skew: Duration,
    /// Audience validation policy for JWTs inside the session cookie.
    pub audience: AudiencePolicy,
    /// JWKS URI override (None = discover from /.well-known/openid-configuration).
    pub jwks_uri: Option<String>,
    /// JWKS cache TTL (default 300s).
    pub jwks_ttl: Duration,
    /// JWT clock skew tolerance applied to exp checks (default 60s).
    pub clock_skew: Duration,
}

impl OidcBrowserSessionConfig {
    /// Convenience constructor — required fields only; all others take defaults.
    pub fn new(
        issuer_url: impl Into<String>,
        client_id: impl Into<String>,
        redirect_uri: impl Into<String>,
        session_key: SessionKey,
        audience: AudiencePolicy,
    ) -> Self {
        Self {
            issuer_url: issuer_url.into(),
            client_id: client_id.into(),
            redirect_uri: redirect_uri.into(),
            session_key,
            callback_path: "/callback".to_string(),
            cookie_name: "session".to_string(),
            session_ttl: Duration::from_secs(8 * 3600),
            refresh_skew: Duration::from_secs(60),
            audience,
            jwks_uri: None,
            jwks_ttl: Duration::from_secs(300),
            clock_skew: Duration::from_secs(60),
        }
    }
}

/// Returned by `oidc_browser_session_layer`. Both parts must be wired in:
///
/// ```ignore
/// let OidcBrowserSessionLayer { layer, callback_router } = oidc_browser_session_layer(cfg)?;
/// let app = Router::new()
///     .merge(callback_router)                           // /callback, /logout
///     .nest("/api/v1", api_routes().layer(api_layer))
///     .fallback_service(ui_handler().layer(layer));
/// ```
pub struct OidcBrowserSessionLayer {
    /// Tower Layer: validates session cookie on every request. Apply to HTML/UI routes.
    pub layer: tower::util::BoxCloneSyncServiceLayer<
        cli_framework::axum::Router,
        cli_framework::axum::http::Request<cli_framework::axum::body::Body>,
        cli_framework::axum::response::Response,
        std::convert::Infallible,
    >,
    /// Axum Router containing /callback and /logout routes (no auth layer applied).
    pub callback_router: cli_framework::axum::Router,
}

/// Build the browser session layer and callback router.
///
/// Validates config at call time (issuer URL, JWKS URI, cookie size budget).
pub fn oidc_browser_session_layer(
    cfg: OidcBrowserSessionConfig,
) -> Result<OidcBrowserSessionLayer, OidcConfigError> {
    let normalized_issuer = crate::normalize_issuer(&cfg.issuer_url)?;

    if let Some(ref uri) = cfg.jwks_uri {
        crate::validate_jwks_uri(uri)?;
    }

    // Startup-time cookie size check
    let cookie_size =
        estimate_cookie_size(cfg.session_key.as_bytes(), DEFAULT_MAX_ACCESS_TOKEN_SIZE);
    if cookie_size > 3900 {
        return Err(OidcConfigError::CookieTooLarge(cookie_size));
    }

    let hmac_key = derive_hmac_key(cfg.session_key.as_bytes());
    let api_audience = cfg.audience.clone();

    let mut cfg = cfg;
    cfg.issuer_url = normalized_issuer;

    let state = Arc::new(BrowserLayerState {
        hmac_key,
        api_audience,
        algorithms: vec![Algorithm::RS256],
        jwks_cache: Mutex::new(JwksCache::empty()),
        discovery: OnceCell::new(),
        last_forced_refetch: Mutex::new(None),
        refetch_gate: Mutex::new(()),
        http: reqwest::Client::builder()
            .user_agent(concat!("cli-framework-oidc/", env!("CARGO_PKG_VERSION")))
            .build()
            .expect("reqwest client"),
        cfg,
    });

    // Build callback router
    use cli_framework::axum::{routing, Router};
    let callback_path = state.cfg.callback_path.clone();
    let callback_router = Router::new()
        .route(&callback_path, routing::get(handlers::handle_callback))
        .route("/logout", routing::post(handlers::handle_logout))
        .with_state(Arc::clone(&state));

    // Build browser session layer
    let browser_layer = layer::BrowserSessionLayer {
        state: Arc::clone(&state),
    };
    let boxed = tower::util::BoxCloneSyncServiceLayer::new(browser_layer);

    Ok(OidcBrowserSessionLayer {
        layer: boxed,
        callback_router,
    })
}

/// Build a dual-mode layer for API routes.
///
/// Accepts `Authorization: Bearer <jwt>` (Agents) or Session Cookie (browser fetch).
/// Bearer takes precedence; an invalid Bearer is a hard reject (cookie not consulted).
/// `api_audience` may differ from the browser session's audience.
pub fn oidc_dual_mode_layer(
    cfg: &OidcBrowserSessionConfig,
    api_audience: AudiencePolicy,
) -> Result<
    tower::util::BoxCloneSyncServiceLayer<
        cli_framework::axum::Router,
        cli_framework::axum::http::Request<cli_framework::axum::body::Body>,
        cli_framework::axum::response::Response,
        std::convert::Infallible,
    >,
    OidcConfigError,
> {
    let normalized_issuer = crate::normalize_issuer(&cfg.issuer_url)?;
    if let Some(ref uri) = cfg.jwks_uri {
        crate::validate_jwks_uri(uri)?;
    }

    let hmac_key = derive_hmac_key(cfg.session_key.as_bytes());

    let mut cfg = cfg.clone();
    cfg.issuer_url = normalized_issuer;

    let state = Arc::new(BrowserLayerState {
        hmac_key,
        api_audience,
        algorithms: vec![Algorithm::RS256],
        jwks_cache: Mutex::new(JwksCache::empty()),
        discovery: OnceCell::new(),
        last_forced_refetch: Mutex::new(None),
        refetch_gate: Mutex::new(()),
        http: reqwest::Client::builder()
            .user_agent(concat!("cli-framework-oidc/", env!("CARGO_PKG_VERSION")))
            .build()
            .expect("reqwest client"),
        cfg,
    });

    let dual_layer = dual::DualModeLayer { state };
    Ok(tower::util::BoxCloneSyncServiceLayer::new(dual_layer))
}
