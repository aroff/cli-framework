//! OIDC server-side validation middleware.

use crate::jwks::{fetch_discovery, fetch_jwks, filter_keys, JwksCache, KeyResult, OidcDiscovery};
use crate::OidcConfigError;
use jsonwebtoken::{Algorithm, DecodingKey, Validation};
use serde_json::Value as JsonValue;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};
use std::time::{Duration, Instant};
use tokio::sync::Mutex;
use tower::{Layer, Service};

// Re-export shared types so callers can use cli_framework_oidc::server::{AudiencePolicy, OidcClaims}.
pub use crate::types::{AudiencePolicy, OidcClaims};

// ── Public config types ─────────────────────────────────────────────────────

#[derive(Clone, Debug)]
pub struct OidcValidationConfig {
    pub issuer_url: String,
    pub audience: AudiencePolicy,
    pub jwks_uri: Option<String>,
    pub algorithms: Vec<Algorithm>,
    pub jwks_ttl: Duration,
    pub clock_skew: Duration,
    pub min_refetch_interval: Duration,
}

impl OidcValidationConfig {
    pub fn new(issuer_url: impl Into<String>, audience: AudiencePolicy) -> Self {
        Self {
            issuer_url: issuer_url.into(),
            audience,
            jwks_uri: None,
            algorithms: vec![Algorithm::RS256],
            jwks_ttl: Duration::from_secs(300),
            clock_skew: Duration::from_secs(60),
            min_refetch_interval: Duration::from_secs(60),
        }
    }
}

// ── Internal state ──────────────────────────────────────────────────────────

struct OidcLayerState {
    issuer_url: String,
    cfg: OidcValidationConfig,
    jwks_cache: Mutex<JwksCache>,
    discovery: tokio::sync::OnceCell<OidcDiscovery>,
    last_forced_refetch: Mutex<Option<Instant>>,
    /// Single-flight gate (ADR 0070): only one task performs a JWKS refetch at a time.
    refetch_gate: Mutex<()>,
    http: reqwest::Client,
}

impl OidcLayerState {
    async fn get_jwks_uri(&self) -> Result<String, String> {
        if let Some(ref uri) = self.cfg.jwks_uri {
            return Ok(uri.clone());
        }
        let disc = self
            .discovery
            .get_or_try_init(|| fetch_discovery(&self.issuer_url, &self.http))
            .await
            .map_err(|e| e.to_string())?;
        Ok(disc.jwks_uri.clone())
    }

    async fn get_decoding_keys(&self, kid: &Option<String>) -> KeyResult {
        // Fast path: fresh cache with the requested kid.
        {
            let cache = self.jwks_cache.lock().await;
            if cache.is_fresh(self.cfg.jwks_ttl) {
                let result = filter_keys(&cache.keys, kid);
                if !matches!(result, KeyResult::UnknownKid) {
                    return result;
                }
            }
        }

        // Single-flight gate: coalesce concurrent refetches.
        let _refetch_guard = self.refetch_gate.lock().await;

        // Double-check after acquiring the gate.
        {
            let cache = self.jwks_cache.lock().await;
            if cache.is_fresh(self.cfg.jwks_ttl) {
                let result = filter_keys(&cache.keys, kid);
                if !matches!(result, KeyResult::UnknownKid) {
                    return result;
                }
            }
        }

        let jwks_uri = match self.get_jwks_uri().await {
            Ok(u) => u,
            Err(e) => {
                tracing::warn!("oidc: failed to get jwks_uri: {e}");
                let cache = self.jwks_cache.lock().await;
                if cache.is_empty() {
                    return KeyResult::Unavailable;
                }
                return filter_keys(&cache.keys, kid);
            }
        };

        // Rate-limit forced refetches.
        {
            let last = self.last_forced_refetch.lock().await;
            if let Some(t) = *last {
                if t.elapsed() < self.cfg.min_refetch_interval {
                    let cache = self.jwks_cache.lock().await;
                    if cache.is_empty() {
                        return KeyResult::Unavailable;
                    }
                    return filter_keys(&cache.keys, kid);
                }
            }
        }

        match fetch_jwks(&jwks_uri, &self.http).await {
            Ok(keys) => {
                let mut cache = self.jwks_cache.lock().await;
                cache.keys = keys;
                cache.fetched_at = Some(Instant::now());
                let mut last = self.last_forced_refetch.lock().await;
                *last = Some(Instant::now());
                filter_keys(&cache.keys, kid)
            }
            Err(e) => {
                tracing::warn!("oidc: jwks fetch failed: {e}");
                let cache = self.jwks_cache.lock().await;
                if cache.is_empty() {
                    return KeyResult::Unavailable;
                }
                filter_keys(&cache.keys, kid)
            }
        }
    }
}

// ── Main entry point ────────────────────────────────────────────────────────

/// Build a tower [`Layer`] that validates JWT bearer tokens on every request.
pub fn oidc_validation_layer(
    cfg: OidcValidationConfig,
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

    if cfg.algorithms.is_empty() {
        return Err(OidcConfigError::EmptyAlgorithms);
    }

    if let Some(ref uri) = cfg.jwks_uri {
        let parsed = url::Url::parse(uri)
            .map_err(|e| OidcConfigError::InvalidJwksUri(format!("{uri}: {e}")))?;
        let scheme = parsed.scheme();
        let host = parsed.host_str().unwrap_or("");
        let is_loopback = host == "127.0.0.1" || host == "localhost" || host == "[::1]";
        if scheme != "https" && !(scheme == "http" && is_loopback) {
            return Err(OidcConfigError::InvalidJwksUri(format!(
                "insecure URI: {uri}"
            )));
        }
    }

    if matches!(cfg.audience, AudiencePolicy::Unchecked) {
        tracing::warn!("oidc_validation_layer: AudiencePolicy::Unchecked — no audience validation");
    }

    let state = Arc::new(OidcLayerState {
        issuer_url: normalized_issuer,
        cfg,
        jwks_cache: Mutex::new(JwksCache::empty()),
        discovery: tokio::sync::OnceCell::new(),
        last_forced_refetch: Mutex::new(None),
        refetch_gate: Mutex::new(()),
        http: reqwest::Client::builder()
            .user_agent(concat!("cli-framework-oidc/", env!("CARGO_PKG_VERSION")))
            .build()
            .expect("reqwest client"),
    });

    let layer = OidcValidationLayer { state };
    Ok(tower::util::BoxCloneSyncServiceLayer::new(layer))
}

// ── Tower Layer / Service impl ───────────────────────────────────────────────

#[derive(Clone)]
struct OidcValidationLayer {
    state: Arc<OidcLayerState>,
}

impl<S> Layer<S> for OidcValidationLayer
where
    S: Service<
            cli_framework::axum::http::Request<cli_framework::axum::body::Body>,
            Response = cli_framework::axum::response::Response,
            Error = std::convert::Infallible,
        > + Clone
        + Send
        + Sync
        + 'static,
    S::Future: Send + 'static,
{
    type Service = OidcValidationService<S>;

    fn layer(&self, inner: S) -> Self::Service {
        OidcValidationService {
            inner,
            state: self.state.clone(),
        }
    }
}

#[derive(Clone)]
struct OidcValidationService<S> {
    inner: S,
    state: Arc<OidcLayerState>,
}

impl<S> Service<cli_framework::axum::http::Request<cli_framework::axum::body::Body>>
    for OidcValidationService<S>
where
    S: Service<
            cli_framework::axum::http::Request<cli_framework::axum::body::Body>,
            Response = cli_framework::axum::response::Response,
            Error = std::convert::Infallible,
        > + Clone
        + Send
        + Sync
        + 'static,
    S::Future: Send + 'static,
{
    type Response = cli_framework::axum::response::Response;
    type Error = std::convert::Infallible;
    type Future = Pin<
        Box<
            dyn Future<
                    Output = Result<
                        cli_framework::axum::response::Response,
                        std::convert::Infallible,
                    >,
                > + Send,
        >,
    >;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.inner.poll_ready(cx)
    }

    fn call(
        &mut self,
        mut req: cli_framework::axum::http::Request<cli_framework::axum::body::Body>,
    ) -> Self::Future {
        let state = self.state.clone();
        let inner = self.inner.clone();
        let mut inner = std::mem::replace(&mut self.inner, inner);

        Box::pin(async move {
            let headers = req.headers().clone();
            match validate_bearer(&headers, &state).await {
                Ok(claims) => {
                    req.extensions_mut().insert(claims);
                    inner.call(req).await
                }
                Err(response) => Ok(response),
            }
        })
    }
}

// ── Axum extractor ──────────────────────────────────────────────────────────

pub struct OidcClaimsRejection;

impl cli_framework::axum::response::IntoResponse for OidcClaimsRejection {
    fn into_response(self) -> cli_framework::axum::response::Response {
        use cli_framework::axum::http::StatusCode;
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            "oidc_validation_layer not installed on this route",
        )
            .into_response()
    }
}

impl<S: Send + Sync> cli_framework::axum::extract::FromRequestParts<S> for OidcClaims {
    type Rejection = OidcClaimsRejection;

    async fn from_request_parts(
        parts: &mut cli_framework::axum::http::request::Parts,
        _state: &S,
    ) -> Result<Self, Self::Rejection> {
        parts
            .extensions
            .get::<OidcClaims>()
            .cloned()
            .ok_or(OidcClaimsRejection)
    }
}

// ── Request validation logic ─────────────────────────────────────────────────

async fn validate_bearer(
    headers: &cli_framework::axum::http::HeaderMap,
    state: &OidcLayerState,
) -> Result<OidcClaims, cli_framework::axum::response::Response> {
    use cli_framework::axum::http::StatusCode;
    use cli_framework::axum::response::IntoResponse;

    let auth_header = headers.get("authorization");
    let raw_token = match auth_header {
        None => {
            return Err((
                StatusCode::UNAUTHORIZED,
                [("www-authenticate", "Bearer")],
                "",
            )
                .into_response())
        }
        Some(h) => {
            let s = h.to_str().unwrap_or("");
            if s.len() <= 7 || !s[..7].eq_ignore_ascii_case("bearer ") {
                return Err((
                    StatusCode::UNAUTHORIZED,
                    [("www-authenticate", "Bearer error=\"invalid_request\"")],
                    "",
                )
                    .into_response());
            }
            &s[7..]
        }
    };

    validate_jwt_token(raw_token, state).await
}

async fn validate_jwt_token(
    raw_token: &str,
    state: &OidcLayerState,
) -> Result<OidcClaims, cli_framework::axum::response::Response> {
    use cli_framework::axum::http::StatusCode;
    use cli_framework::axum::response::IntoResponse;

    let header = match jsonwebtoken::decode_header(raw_token) {
        Ok(h) => h,
        Err(_) => {
            return Err((
                StatusCode::UNAUTHORIZED,
                [("www-authenticate", "Bearer error=\"invalid_token\"")],
                "",
            )
                .into_response())
        }
    };

    if !state.cfg.algorithms.contains(&header.alg) {
        return Err((
            StatusCode::UNAUTHORIZED,
            [(
                "www-authenticate",
                "Bearer error=\"invalid_token\", error_description=\"unsupported_algorithm\"",
            )],
            "",
        )
            .into_response());
    }

    let keys = match state.get_decoding_keys(&header.kid).await {
        KeyResult::Keys(k) => k,
        KeyResult::Unavailable => {
            return Err(cli_framework::axum::http::Response::builder()
                .status(StatusCode::SERVICE_UNAVAILABLE)
                .header(
                    "retry-after",
                    state.cfg.min_refetch_interval.as_secs().to_string(),
                )
                .body(cli_framework::axum::body::Body::from("JWKS unavailable"))
                .unwrap());
        }
        KeyResult::UnknownKid => {
            return Err((
                StatusCode::UNAUTHORIZED,
                [(
                    "www-authenticate",
                    "Bearer error=\"invalid_token\", error_description=\"unknown_key\"",
                )],
                "",
            )
                .into_response());
        }
    };

    let mut last_error: Option<String> = None;
    for key in &keys {
        match try_validate_jwt(raw_token, key, &state.cfg, &state.issuer_url) {
            Ok(claims) => return Ok(claims),
            Err(e) => {
                last_error = Some(e);
            }
        }
    }

    let reason = last_error.unwrap_or_else(|| "invalid_token".to_string());
    Err((
        StatusCode::UNAUTHORIZED,
        [(
            "www-authenticate",
            format!("Bearer error=\"invalid_token\", error_description=\"{reason}\""),
        )],
        "",
    )
        .into_response())
}

pub(crate) fn try_validate_jwt(
    token: &str,
    key: &DecodingKey,
    cfg: &OidcValidationConfig,
    issuer_url: &str,
) -> Result<OidcClaims, String> {
    let mut validation = Validation::new(cfg.algorithms[0]);
    validation.algorithms = cfg.algorithms.clone();
    validation.set_issuer(&[issuer_url]);
    match &cfg.audience {
        AudiencePolicy::Require(aud) => {
            validation.set_audience(&[aud]);
        }
        AudiencePolicy::RequireAny(auds) => {
            validation.set_audience(auds);
        }
        AudiencePolicy::Unchecked => {
            validation.validate_aud = false;
        }
    }
    validation.leeway = cfg.clock_skew.as_secs();

    let token_data = jsonwebtoken::decode::<JsonValue>(token, key, &validation)
        .map_err(|e| crate::jwks::map_jwt_error(&e))?;

    let claims = &token_data.claims;

    let sub = claims["sub"]
        .as_str()
        .ok_or_else(|| "malformed_token".to_string())?
        .to_string();
    let iss = claims["iss"].as_str().unwrap_or("").to_string();
    let exp = claims["exp"].as_i64().unwrap_or(0);
    let iat = claims["iat"].as_i64();
    let nbf = claims["nbf"].as_i64();

    let aud: Vec<String> = match &claims["aud"] {
        JsonValue::String(s) => vec![s.clone()],
        JsonValue::Array(arr) => arr
            .iter()
            .filter_map(|v| v.as_str().map(String::from))
            .collect(),
        _ => vec![],
    };

    let preferred_username = claims["preferred_username"].as_str().map(String::from);
    let email = claims["email"].as_str().map(String::from);

    let scopes: Vec<String> = if let Some(s) = claims["scope"].as_str() {
        s.split_whitespace().map(String::from).collect()
    } else if let Some(arr) = claims["scp"].as_array() {
        arr.iter()
            .filter_map(|v| v.as_str().map(String::from))
            .collect()
    } else {
        vec![]
    };

    let roles: Vec<String> = claims["realm_access"]["roles"]
        .as_array()
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default();

    Ok(OidcClaims {
        sub,
        iss,
        aud,
        exp,
        iat,
        nbf,
        preferred_username,
        email,
        scopes,
        roles,
        raw: claims.clone(),
    })
}
