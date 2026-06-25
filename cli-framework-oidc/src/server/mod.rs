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

// ── Error types ─────────────────────────────────────────────────────────────

/// Why a token that was extracted from the request failed verification.
///
/// `#[non_exhaustive]` reserves room for future additions (e.g. enabling `nbf`
/// validation would make `NotYetValid` a live path).
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum TokenRejection {
    /// `jsonwebtoken::decode_header` failed — not even a parseable JWT.
    /// Emits NO `error_description` on the wire (distinct from `Malformed`).
    Undecodable,
    /// The header `alg` is not in the configured `algorithms` set.
    UnsupportedAlgorithm,
    /// The header `kid` matched no key in the (refetched) JWKS.
    UnknownKey,
    /// Decoded but unusable: missing `sub`, or any `jsonwebtoken` error not otherwise modelled.
    /// Emits `error_description="malformed_token"`.
    Malformed,
    /// `exp` is in the past (beyond `clock_skew`).
    Expired,
    /// `nbf` is in the future (beyond `clock_skew`). Reserved -- not produced today.
    NotYetValid,
    /// Signature did not verify against the selected key.
    InvalidSignature,
    /// `iss` did not match the configured issuer.
    InvalidIssuer,
    /// `aud` did not satisfy the configured `AudiencePolicy`.
    InvalidAudience,
}

impl std::fmt::Display for TokenRejection {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Undecodable => write!(f, "token could not be decoded as a JWT"),
            Self::UnsupportedAlgorithm => write!(f, "token uses an unsupported signing algorithm"),
            Self::UnknownKey => write!(f, "token key ID not found in JWKS"),
            Self::Malformed => write!(f, "token is malformed or missing required claims"),
            Self::Expired => write!(f, "token has expired"),
            Self::NotYetValid => write!(f, "token is not yet valid"),
            Self::InvalidSignature => write!(f, "token signature is invalid"),
            Self::InvalidIssuer => write!(f, "token issuer does not match"),
            Self::InvalidAudience => write!(f, "token audience does not match"),
        }
    }
}

/// Outcome of verifying a token outside the HTTP layer.
///
/// `#[non_exhaustive]` allows adding variants in minor versions.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum OidcValidationError {
    /// No credential was offered: the `Authorization` header was absent (only
    /// returned by `validate_authorization` when the header is `None`).
    /// Maps to `401` + `WWW-Authenticate: Bearer`.
    MissingToken,

    /// A credential was offered but is not a well-formed `Bearer <token>`.
    /// A present-but-non-UTF-8 header is *malformed*, not *missing*.
    /// Maps to `401` + `Bearer error="invalid_request"`.
    MalformedAuthorization,

    /// A token was extracted and rejected.
    /// Maps to `401` + `Bearer error="invalid_token"[, error_description="<reason>"]`.
    InvalidToken(TokenRejection),

    /// JWKS could not be fetched and no usable cached keys exist.
    /// Maps to `503` + `Retry-After: <min_refetch_interval secs>`.
    JwksUnavailable,
}

impl std::fmt::Display for OidcValidationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::MissingToken => write!(f, "no Authorization header present"),
            Self::MalformedAuthorization => {
                write!(f, "Authorization header is not a valid Bearer token")
            }
            Self::InvalidToken(r) => write!(f, "token rejected: {r}"),
            Self::JwksUnavailable => write!(f, "JWKS unavailable, cannot verify token"),
        }
    }
}

impl std::error::Error for OidcValidationError {}

// ── Public config types ─────────────────────────────────────────────────────────

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

// ── OidcValidator ───────────────────────────────────────────────────────────

/// A cloneable, `Send + Sync` handle for verifying OIDC JWT tokens.
///
/// Clones share the same underlying JWKS cache, discovery state, and
/// single-flight refetch gate (ADR 0070). Construct via [`OidcValidator::new`]
/// and call [`validate`](OidcValidator::validate) or
/// [`validate_authorization`](OidcValidator::validate_authorization).
#[derive(Clone)]
pub struct OidcValidator {
    state: Arc<OidcLayerState>,
}

impl OidcValidator {
    /// Build a validator. Performs config validation (issuer normalization,
    /// non-empty `algorithms`, JWKS-URI scheme check, `Unchecked` audience WARN).
    pub fn new(cfg: OidcValidationConfig) -> Result<Self, OidcConfigError> {
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
            tracing::warn!(
                "oidc_validation_layer: AudiencePolicy::Unchecked -- no audience validation"
            );
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

        Ok(Self { state })
    }

    /// Verify an already-extracted bearer token (no `Bearer ` prefix, no header
    /// parsing). This is the primary seam for trait-based consumers.
    pub async fn validate(&self, token: &str) -> Result<OidcClaims, OidcValidationError> {
        let header = match jsonwebtoken::decode_header(token) {
            Ok(h) => h,
            Err(_) => {
                return Err(OidcValidationError::InvalidToken(
                    TokenRejection::Undecodable,
                ))
            }
        };

        if !self.state.cfg.algorithms.contains(&header.alg) {
            return Err(OidcValidationError::InvalidToken(
                TokenRejection::UnsupportedAlgorithm,
            ));
        }

        let keys = match self.state.get_decoding_keys(&header.kid).await {
            KeyResult::Keys(k) => k,
            KeyResult::Unavailable => return Err(OidcValidationError::JwksUnavailable),
            KeyResult::UnknownKid => {
                return Err(OidcValidationError::InvalidToken(
                    TokenRejection::UnknownKey,
                ))
            }
        };

        let mut last_rejection: Option<TokenRejection> = None;
        for key in &keys {
            match try_validate_jwt(token, key, &self.state.cfg, &self.state.issuer_url) {
                Ok(claims) => return Ok(claims),
                Err(r) => {
                    last_rejection = Some(r);
                }
            }
        }

        // `KeyResult::Keys` always carries >= 1 key (`filter_keys` never yields an
        // empty `Keys`), so the loop body runs at least once.
        Err(OidcValidationError::InvalidToken(
            last_rejection.unwrap_or_else(|| unreachable!("keys vec was non-empty")),
        ))
    }

    /// Parse an `Authorization` header value and verify the token.
    ///
    /// - `None` => [`OidcValidationError::MissingToken`]
    /// - A value that is not `Bearer <token>` (scheme matched ASCII-case-insensitively)
    ///   => [`OidcValidationError::MalformedAuthorization`]
    /// - Otherwise delegates to [`validate`](OidcValidator::validate).
    pub async fn validate_authorization(
        &self,
        authorization: Option<&str>,
    ) -> Result<OidcClaims, OidcValidationError> {
        let s = match authorization {
            None => return Err(OidcValidationError::MissingToken),
            Some(s) => s,
        };
        if s.len() <= 7 || !s[..7].eq_ignore_ascii_case("bearer ") {
            return Err(OidcValidationError::MalformedAuthorization);
        }
        self.validate(&s[7..]).await
    }

    /// In-crate accessor used by `error_to_response` to read `min_refetch_interval`
    /// (for `Retry-After`) and audience policy.
    pub(crate) fn config(&self) -> &OidcValidationConfig {
        &self.state.cfg
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
    let validator = OidcValidator::new(cfg)?;
    let layer = OidcValidationLayer { validator };
    Ok(tower::util::BoxCloneSyncServiceLayer::new(layer))
}

// ── Tower Layer / Service impl ───────────────────────────────────────────────

#[derive(Clone)]
struct OidcValidationLayer {
    validator: OidcValidator,
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
            validator: self.validator.clone(),
        }
    }
}

#[derive(Clone)]
struct OidcValidationService<S> {
    inner: S,
    validator: OidcValidator,
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
        let validator = self.validator.clone();
        let inner = self.inner.clone();
        let mut inner = std::mem::replace(&mut self.inner, inner);

        Box::pin(async move {
            // Present-but-non-UTF-8 header -> Some("") so it flows to MalformedAuthorization
            // (a broken credential is *malformed*, not *missing*) -- byte-identical to today.
            let auth: Option<String> = req
                .headers()
                .get("authorization")
                .map(|h| h.to_str().unwrap_or("").to_owned());
            match validator.validate_authorization(auth.as_deref()).await {
                Ok(claims) => {
                    req.extensions_mut().insert(claims);
                    inner.call(req).await
                }
                Err(e) => Ok(error_to_response(&e, validator.config())),
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

// ── Error -> HTTP response mapping (sole place building HTTP responses) ───────

fn error_to_response(
    err: &OidcValidationError,
    cfg: &OidcValidationConfig,
) -> cli_framework::axum::response::Response {
    use cli_framework::axum::http::StatusCode;
    use cli_framework::axum::response::IntoResponse;

    match err {
        OidcValidationError::MissingToken => (
            StatusCode::UNAUTHORIZED,
            [("www-authenticate", "Bearer".to_owned())],
            "",
        )
            .into_response(),

        OidcValidationError::MalformedAuthorization => (
            StatusCode::UNAUTHORIZED,
            [(
                "www-authenticate",
                "Bearer error=\"invalid_request\"".to_owned(),
            )],
            "",
        )
            .into_response(),

        OidcValidationError::InvalidToken(TokenRejection::Undecodable) => (
            StatusCode::UNAUTHORIZED,
            [(
                "www-authenticate",
                "Bearer error=\"invalid_token\"".to_owned(),
            )],
            "",
        )
            .into_response(),

        OidcValidationError::InvalidToken(rejection) => {
            let desc = rejection_wire_string(rejection);
            (
                StatusCode::UNAUTHORIZED,
                [(
                    "www-authenticate",
                    format!("Bearer error=\"invalid_token\", error_description=\"{desc}\""),
                )],
                "",
            )
                .into_response()
        }

        OidcValidationError::JwksUnavailable => cli_framework::axum::http::Response::builder()
            .status(StatusCode::SERVICE_UNAVAILABLE)
            .header(
                "retry-after",
                cfg.min_refetch_interval.as_secs().to_string(),
            )
            .body(cli_framework::axum::body::Body::from("JWKS unavailable"))
            .unwrap(),
    }
}

/// Map a `TokenRejection` variant to the wire `error_description` string.
/// `Undecodable` is handled separately in `error_to_response` and MUST NOT reach here.
fn rejection_wire_string(r: &TokenRejection) -> &'static str {
    match r {
        TokenRejection::Undecodable => {
            unreachable!("Undecodable handled separately in error_to_response")
        }
        TokenRejection::UnsupportedAlgorithm => "unsupported_algorithm",
        TokenRejection::UnknownKey => "unknown_key",
        TokenRejection::Malformed => "malformed_token",
        TokenRejection::Expired => "expired",
        TokenRejection::NotYetValid => "not_yet_valid",
        TokenRejection::InvalidSignature => "invalid_signature",
        TokenRejection::InvalidIssuer => "invalid_issuer",
        TokenRejection::InvalidAudience => "invalid_audience",
    }
}

// ── JWT error -> TokenRejection ───────────────────────────────────────────────

fn jwt_err_to_rejection(e: &jsonwebtoken::errors::Error) -> TokenRejection {
    use jsonwebtoken::errors::ErrorKind;
    match e.kind() {
        ErrorKind::ExpiredSignature => TokenRejection::Expired,
        ErrorKind::ImmatureSignature => TokenRejection::NotYetValid,
        ErrorKind::InvalidSignature => TokenRejection::InvalidSignature,
        ErrorKind::InvalidIssuer => TokenRejection::InvalidIssuer,
        ErrorKind::InvalidAudience => TokenRejection::InvalidAudience,
        ErrorKind::InvalidAlgorithm => TokenRejection::UnsupportedAlgorithm,
        _ => TokenRejection::Malformed,
    }
}

// ── Per-key JWT verification ─────────────────────────────────────────────────

pub(crate) fn try_validate_jwt(
    token: &str,
    key: &DecodingKey,
    cfg: &OidcValidationConfig,
    issuer_url: &str,
) -> Result<OidcClaims, TokenRejection> {
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
        .map_err(|e| jwt_err_to_rejection(&e))?;

    let claims = &token_data.claims;

    let sub = claims["sub"]
        .as_str()
        .ok_or(TokenRejection::Malformed)?
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
