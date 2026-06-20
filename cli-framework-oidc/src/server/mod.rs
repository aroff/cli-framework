//! OIDC server-side validation middleware.

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

#[derive(Clone, Debug)]
pub enum AudiencePolicy {
    Require(String),
    Unchecked,
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

/// Extracted and validated OIDC claims, inserted into request extensions.
#[derive(Clone, Debug)]
pub struct OidcClaims {
    pub sub: String,
    pub iss: String,
    pub aud: Vec<String>,
    pub exp: i64,
    pub iat: Option<i64>,
    pub nbf: Option<i64>,
    pub preferred_username: Option<String>,
    pub email: Option<String>,
    pub scopes: Vec<String>,
    pub roles: Vec<String>,
    pub raw: JsonValue,
}

// ── Internal state ──────────────────────────────────────────────────────────

struct JwksCache {
    keys: Vec<(Option<String>, DecodingKey)>, // (kid, key)
    fetched_at: Option<Instant>,
}

impl JwksCache {
    fn empty() -> Self {
        Self {
            keys: vec![],
            fetched_at: None,
        }
    }

    fn is_fresh(&self, ttl: Duration) -> bool {
        self.fetched_at.is_some_and(|t| t.elapsed() < ttl)
    }

    fn is_empty(&self) -> bool {
        self.keys.is_empty()
    }
}

struct OidcDiscovery {
    jwks_uri: String,
}

struct OidcLayerState {
    issuer_url: String,
    cfg: OidcValidationConfig,
    jwks_cache: Mutex<JwksCache>,
    discovery: tokio::sync::OnceCell<OidcDiscovery>,
    last_forced_refetch: Mutex<Option<Instant>>,
    http: reqwest::Client,
}

impl OidcLayerState {
    async fn get_jwks_uri(&self) -> Result<String, String> {
        if let Some(ref uri) = self.cfg.jwks_uri {
            return Ok(uri.clone());
        }
        let disc = self
            .discovery
            .get_or_try_init(|| fetch_discovery_jwks(&self.issuer_url, &self.http))
            .await
            .map_err(|e| e.to_string())?;
        Ok(disc.jwks_uri.clone())
    }

    async fn get_decoding_keys(&self, kid: &Option<String>) -> KeyResult {
        {
            let cache = self.jwks_cache.lock().await;
            if cache.is_fresh(self.cfg.jwks_ttl) {
                return filter_keys(&cache.keys, kid);
            }
        }

        // Try to refetch
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

        // Check min_refetch_interval
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

enum KeyResult {
    Keys(Vec<DecodingKey>),
    Unavailable,
    UnknownKid,
}

fn filter_keys(all: &[(Option<String>, DecodingKey)], kid: &Option<String>) -> KeyResult {
    if all.is_empty() {
        return KeyResult::Unavailable;
    }
    let matching: Vec<DecodingKey> = match kid {
        Some(k) => all
            .iter()
            .filter(|(id, _)| id.as_deref() == Some(k.as_str()))
            .map(|(_, key)| key.clone())
            .collect(),
        None => {
            if all.len() == 1 {
                all.iter().map(|(_, key)| key.clone()).collect()
            } else {
                return KeyResult::UnknownKid;
            }
        }
    };
    if matching.is_empty() && kid.is_some() {
        // Unknown kid — could be stale cache; caller may force refetch
        KeyResult::UnknownKid
    } else if matching.is_empty() {
        KeyResult::Unavailable
    } else {
        KeyResult::Keys(matching)
    }
}

// ── Main entry point ────────────────────────────────────────────────────────

/// Build a tower [`Layer`] that validates JWT bearer tokens on every request.
///
/// Returns a `BoxCloneSyncServiceLayer` compatible with `cli_framework::tower::util::BoxCloneLayer<axum::Router>`.
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
            // Extract headers before the async boundary to avoid holding a &Request<Body>
            // across await (Body is not Sync).
            let headers = req.headers().clone();
            match validate_request(&headers, &state).await {
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

async fn validate_request(
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

    // Decode header (no signature verification)
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

    // Reject algorithms that provide no security (HS256 etc. with None key).
    // In jsonwebtoken 9, there is no Algorithm::None variant; we rely on
    // the allowlist check below to reject anything not in cfg.algorithms.
    // No explicit None check needed.

    if !state.cfg.algorithms.contains(&header.alg) {
        return Err((
            StatusCode::UNAUTHORIZED,
            [(
                "www-authenticate",
                "Bearer error=\"invalid_token\",error_description=\"unsupported_algorithm\"",
            )],
            "",
        )
            .into_response());
    }

    // Get matching keys
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
            // Force a refetch once then try again
            // (simplified: just return 401)
            return Err((
                StatusCode::UNAUTHORIZED,
                [(
                    "www-authenticate",
                    "Bearer error=\"invalid_token\",error_description=\"unknown_key\"",
                )],
                "",
            )
                .into_response());
        }
    };

    // Try each key
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
            format!("Bearer error=\"invalid_token\",error_description=\"{reason}\""),
        )],
        "",
    )
        .into_response())
}

fn try_validate_jwt(
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
        AudiencePolicy::Unchecked => {
            validation.validate_aud = false;
        }
    }
    validation.leeway = cfg.clock_skew.as_secs();

    let token_data =
        jsonwebtoken::decode::<JsonValue>(token, key, &validation).map_err(|e| e.to_string())?;

    let claims = &token_data.claims;

    let sub = claims["sub"].as_str().unwrap_or("").to_string();
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

// ── JWKS fetching ────────────────────────────────────────────────────────────

async fn fetch_discovery_jwks(
    issuer_url: &str,
    http: &reqwest::Client,
) -> Result<OidcDiscovery, String> {
    let url = format!("{}/.well-known/openid-configuration", issuer_url);
    let resp = http.get(&url).send().await.map_err(|e| e.to_string())?;
    let doc: JsonValue = resp.json().await.map_err(|e| e.to_string())?;
    let jwks_uri = doc["jwks_uri"]
        .as_str()
        .ok_or_else(|| "missing jwks_uri in discovery doc".to_string())?
        .to_string();
    Ok(OidcDiscovery { jwks_uri })
}

async fn fetch_jwks(
    jwks_uri: &str,
    http: &reqwest::Client,
) -> Result<Vec<(Option<String>, DecodingKey)>, String> {
    let resp = http.get(jwks_uri).send().await.map_err(|e| e.to_string())?;
    let doc: JsonValue = resp.json().await.map_err(|e| e.to_string())?;

    let keys_arr = doc["keys"].as_array().ok_or("missing keys array")?;
    let mut result = vec![];

    for jwk in keys_arr {
        let kid = jwk["kid"].as_str().map(String::from);
        let kty = jwk["kty"].as_str().unwrap_or("");

        let key = match kty {
            "RSA" => {
                let n = jwk["n"].as_str().unwrap_or("");
                let e = jwk["e"].as_str().unwrap_or("");
                DecodingKey::from_rsa_components(n, e).map_err(|e| e.to_string())?
            }
            "EC" => {
                let x = jwk["x"].as_str().unwrap_or("");
                let y = jwk["y"].as_str().unwrap_or("");
                DecodingKey::from_ec_components(x, y).map_err(|e| e.to_string())?
            }
            _ => continue, // skip unsupported key types
        };

        result.push((kid, key));
    }

    Ok(result)
}
