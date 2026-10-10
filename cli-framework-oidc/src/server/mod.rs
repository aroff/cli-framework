//! OIDC server-side validation middleware.

use crate::claim_path::ClaimPath;
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

pub use crate::claim_path::DEFAULT_ROLES_CLAIM_PATH;
/// Inline key set for [`OidcValidationConfig::static_jwks`] (re-exported from
/// `jsonwebtoken` so callers need not depend on it directly).
pub use jsonwebtoken::jwk::JwkSet;

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
    /// `iss` is missing or did not match the configured issuer.
    InvalidIssuer,
    /// `aud` is missing or did not satisfy the configured `AudiencePolicy`.
    InvalidAudience,
    /// The token's `iss` names none of the issuers a multi-issuer validator
    /// trusts (ADR 0082). Decided before any key lookup or JWKS fetch.
    UnknownIssuer,
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
            Self::UnknownIssuer => write!(f, "token issuer is not trusted"),
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

/// How to verify tokens from one issuer.
///
/// [`OidcValidator::new`] trusts exactly one issuer; [`OidcValidator::new_multi`]
/// takes one of these per trusted issuer, each with its own audience policy,
/// key source, algorithms and claim paths.
///
/// Prefer [`OidcValidationConfig::new`] plus field updates (or the `with_*`
/// methods) over a struct literal, so new fields keep their defaults.
#[derive(Clone, Debug)]
pub struct OidcValidationConfig {
    /// The issuer, normalized with [`crate::normalize_issuer`]. A token's `iss`
    /// must equal the normalized value exactly.
    pub issuer_url: String,
    pub audience: AudiencePolicy,
    /// JWKS endpoint. `None` discovers it from
    /// `{issuer_url}/.well-known/openid-configuration`. Must be `None` when
    /// `static_jwks` is set.
    pub jwks_uri: Option<String>,
    pub algorithms: Vec<Algorithm>,
    pub jwks_ttl: Duration,
    pub clock_skew: Duration,
    pub min_refetch_interval: Duration,
    /// Keys given inline instead of fetched. When set, the validator never
    /// performs discovery or a JWKS fetch for this issuer: a `kid` missing
    /// from this set is rejected as [`TokenRejection::UnknownKey`]. Meant for
    /// tests and for issuers whose keys are distributed out of band.
    pub static_jwks: Option<JwkSet>,
    /// Where to read [`OidcClaims::roles`] from. Dot-separated object keys,
    /// `\.` for a literal dot in a key (see the crate README). Default
    /// [`DEFAULT_ROLES_CLAIM_PATH`].
    pub roles_claim_path: String,
    /// Where to read [`OidcClaims::groups`] from, same syntax as
    /// `roles_claim_path`. `None` (the default) leaves `groups` empty.
    pub groups_claim_path: Option<String>,
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
            static_jwks: None,
            roles_claim_path: DEFAULT_ROLES_CLAIM_PATH.to_string(),
            groups_claim_path: None,
        }
    }

    /// Use `jwks` as this issuer's keys instead of fetching them.
    pub fn with_static_jwks(mut self, jwks: JwkSet) -> Self {
        self.static_jwks = Some(jwks);
        self
    }

    /// Read roles from `path` instead of [`DEFAULT_ROLES_CLAIM_PATH`].
    pub fn with_roles_claim_path(mut self, path: impl Into<String>) -> Self {
        self.roles_claim_path = path.into();
        self
    }

    /// Read groups from `path`.
    pub fn with_groups_claim_path(mut self, path: impl Into<String>) -> Self {
        self.groups_claim_path = Some(path.into());
        self
    }
}

// ── Internal state ──────────────────────────────────────────────────────────

/// Everything needed to verify tokens from one trusted issuer. Each issuer
/// owns its own JWKS cache, discovery state, refetch rate-limit and
/// single-flight gate, so one issuer's keys never verify another's tokens and
/// one issuer's refetches never consume another's budget.
struct IssuerState {
    issuer_url: String,
    cfg: OidcValidationConfig,
    roles_path: ClaimPath,
    groups_path: Option<ClaimPath>,
    /// `Some` when the keys were configured inline; nothing is fetched then.
    static_keys: Option<Vec<(Option<String>, DecodingKey)>>,
    jwks_cache: Mutex<JwksCache>,
    discovery: tokio::sync::OnceCell<OidcDiscovery>,
    last_forced_refetch: Mutex<Option<Instant>>,
    /// Single-flight gate (ADR 0070): only one task performs a JWKS refetch at a time.
    refetch_gate: Mutex<()>,
    http: reqwest::Client,
}

impl IssuerState {
    /// Validate one issuer's config: issuer normalization, non-empty
    /// `algorithms`, JWKS-URI scheme check, key source, claim paths, and the
    /// `Unchecked` audience WARN (unless `warn_unchecked` is false: the caller
    /// binds its tokens some other way and says so itself).
    fn build(
        cfg: OidcValidationConfig,
        http: reqwest::Client,
        warn_unchecked: bool,
    ) -> Result<Self, OidcConfigError> {
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

        let static_keys = match &cfg.static_jwks {
            None => None,
            Some(_) if cfg.jwks_uri.is_some() => {
                return Err(OidcConfigError::InvalidJwks(format!(
                    "{normalized_issuer}: static_jwks and jwks_uri are mutually exclusive"
                )));
            }
            Some(set) => Some(static_decoding_keys(&normalized_issuer, set)?),
        };

        let roles_path = ClaimPath::parse(&cfg.roles_claim_path)?;
        let groups_path = cfg
            .groups_claim_path
            .as_deref()
            .map(ClaimPath::parse)
            .transpose()?;

        if warn_unchecked && matches!(cfg.audience, AudiencePolicy::Unchecked) {
            tracing::warn!(
                issuer = %normalized_issuer,
                "oidc_validation_layer: AudiencePolicy::Unchecked -- no audience validation"
            );
        }

        Ok(Self {
            issuer_url: normalized_issuer,
            cfg,
            roles_path,
            groups_path,
            static_keys,
            jwks_cache: Mutex::new(JwksCache::empty()),
            discovery: tokio::sync::OnceCell::new(),
            last_forced_refetch: Mutex::new(None),
            refetch_gate: Mutex::new(()),
            http,
        })
    }

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
        if let Some(ref keys) = self.static_keys {
            return filter_keys(keys, kid);
        }

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

    /// Verify `token` (whose header is `header`) against this issuer only.
    async fn validate(
        &self,
        token: &str,
        header: &jsonwebtoken::Header,
    ) -> Result<OidcClaims, OidcValidationError> {
        if !self.cfg.algorithms.contains(&header.alg) {
            return Err(OidcValidationError::InvalidToken(
                TokenRejection::UnsupportedAlgorithm,
            ));
        }

        let keys = match self.get_decoding_keys(&header.kid).await {
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
            match try_validate_jwt(token, key, self) {
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
}

/// Convert an inline JWK set into decoding keys. Symmetric (`oct`) keys are
/// refused: a key set published for verification carries public keys only.
fn static_decoding_keys(
    issuer: &str,
    set: &JwkSet,
) -> Result<Vec<(Option<String>, DecodingKey)>, OidcConfigError> {
    use jsonwebtoken::jwk::AlgorithmParameters;
    if set.keys.is_empty() {
        return Err(OidcConfigError::InvalidJwks(format!(
            "{issuer}: static_jwks has no keys"
        )));
    }
    set.keys
        .iter()
        .map(|jwk| {
            if matches!(jwk.algorithm, AlgorithmParameters::OctetKey(_)) {
                return Err(OidcConfigError::InvalidJwks(format!(
                    "{issuer}: symmetric (oct) keys are not accepted"
                )));
            }
            let key = DecodingKey::from_jwk(jwk)
                .map_err(|e| OidcConfigError::InvalidJwks(format!("{issuer}: {e}")))?;
            Ok((jwk.common.key_id.clone(), key))
        })
        .collect()
}

/// The `iss` claim of `token`, read WITHOUT verifying anything. Used only to
/// choose which trusted issuer's config verifies the token; the chosen
/// issuer then checks the signature, `iss`, `aud` and `exp` in full.
/// `Err` when the payload is not base64url JSON; `Ok(None)` when it has no
/// string `iss`.
fn unverified_issuer(token: &str) -> Result<Option<String>, ()> {
    use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
    let payload = token.split('.').nth(1).ok_or(())?;
    let bytes = URL_SAFE_NO_PAD.decode(payload).map_err(|_| ())?;
    let claims: JsonValue = serde_json::from_slice(&bytes).map_err(|_| ())?;
    Ok(claims
        .get("iss")
        .and_then(JsonValue::as_str)
        .map(String::from))
}

// ── OidcValidator ───────────────────────────────────────────────────────────

struct ValidatorInner {
    issuers: Vec<IssuerState>,
    /// `true` for [`OidcValidator::new_multi`]: pick the issuer by the token's
    /// unverified `iss`. `false` for [`OidcValidator::new`]: the single issuer
    /// verifies every token, exactly as before multi-issuer support.
    select_by_iss: bool,
}

/// A cloneable, `Send + Sync` handle for verifying OIDC JWT tokens.
///
/// Clones share the same underlying JWKS caches, discovery state, and
/// single-flight refetch gates (ADR 0070). Construct via [`OidcValidator::new`]
/// (one issuer) or [`OidcValidator::new_multi`] (several, ADR 0082) and call
/// [`validate`](OidcValidator::validate) or
/// [`validate_authorization`](OidcValidator::validate_authorization).
#[derive(Clone)]
pub struct OidcValidator {
    inner: Arc<ValidatorInner>,
}

impl OidcValidator {
    /// Build a validator for one issuer. Performs config validation (issuer
    /// normalization, non-empty `algorithms`, JWKS-URI scheme check, key
    /// source, claim paths, `Unchecked` audience WARN).
    pub fn new(cfg: OidcValidationConfig) -> Result<Self, OidcConfigError> {
        Self::new_single(cfg, true)
    }

    /// [`new`](Self::new) without the `Unchecked` audience WARN, for a caller
    /// that binds its tokens to itself another way (the host session's `azp`
    /// check) and would otherwise log a warning that isn't true of it.
    #[cfg(feature = "host-session")]
    pub(crate) fn new_without_audience_warning(
        cfg: OidcValidationConfig,
    ) -> Result<Self, OidcConfigError> {
        Self::new_single(cfg, false)
    }

    fn new_single(
        cfg: OidcValidationConfig,
        warn_unchecked: bool,
    ) -> Result<Self, OidcConfigError> {
        let issuer = IssuerState::build(cfg, http_client(), warn_unchecked)?;
        Ok(Self {
            inner: Arc::new(ValidatorInner {
                issuers: vec![issuer],
                select_by_iss: false,
            }),
        })
    }

    /// Build a validator that trusts several issuers (ADR 0082).
    ///
    /// Each config is validated as in [`new`](Self::new). A token is routed by
    /// its `iss` claim, read from the unverified payload, to the issuer whose
    /// normalized `issuer_url` equals it exactly; that issuer then verifies the
    /// signature (with its own keys), `iss`, `aud` and `exp`. A token
    /// whose `iss` matches no configured issuer is rejected with
    /// [`TokenRejection::UnknownIssuer`] before any discovery or JWKS fetch.
    ///
    /// Errors: [`OidcConfigError::MissingField`]`("issuers")` for an empty
    /// list, [`OidcConfigError::DuplicateIssuer`] when two configs normalize to
    /// the same issuer.
    pub fn new_multi(
        cfgs: impl IntoIterator<Item = OidcValidationConfig>,
    ) -> Result<Self, OidcConfigError> {
        let http = http_client();
        let mut issuers: Vec<IssuerState> = Vec::new();
        for cfg in cfgs {
            let issuer = IssuerState::build(cfg, http.clone(), true)?;
            if issuers.iter().any(|i| i.issuer_url == issuer.issuer_url) {
                return Err(OidcConfigError::DuplicateIssuer(issuer.issuer_url));
            }
            issuers.push(issuer);
        }
        if issuers.is_empty() {
            return Err(OidcConfigError::MissingField("issuers"));
        }
        Ok(Self {
            inner: Arc::new(ValidatorInner {
                issuers,
                select_by_iss: true,
            }),
        })
    }

    /// The normalized issuers this validator trusts, in configuration order.
    pub fn issuers(&self) -> impl Iterator<Item = &str> {
        self.inner.issuers.iter().map(|i| i.issuer_url.as_str())
    }

    /// Verify an already-extracted bearer token (no `Bearer ` prefix, no header
    /// parsing). This is the primary seam for trait-based consumers.
    pub async fn validate(&self, token: &str) -> Result<OidcClaims, OidcValidationError> {
        self.validate_inner(token).await.map_err(|(e, _)| e)
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
        self.authorize_inner(authorization)
            .await
            .map_err(|(e, _)| e)
    }

    /// A tower [`Layer`] backed by this validator — the multi-issuer
    /// counterpart of [`oidc_validation_layer`]. The layer shares this
    /// validator's caches.
    pub fn layer(&self) -> BoxedOidcLayer {
        tower::util::BoxCloneSyncServiceLayer::new(OidcValidationLayer {
            validator: self.clone(),
        })
    }

    /// Like `validate_authorization`, but a rejection also carries the
    /// `Retry-After` to advertise if it is [`OidcValidationError::JwksUnavailable`].
    async fn authorize_inner(
        &self,
        authorization: Option<&str>,
    ) -> Result<OidcClaims, (OidcValidationError, Duration)> {
        let fallback = self.inner.issuers[0].cfg.min_refetch_interval;
        let s = match authorization {
            None => return Err((OidcValidationError::MissingToken, fallback)),
            Some(s) => s,
        };
        if s.len() <= 7 || !s[..7].eq_ignore_ascii_case("bearer ") {
            return Err((OidcValidationError::MalformedAuthorization, fallback));
        }
        self.validate_inner(&s[7..]).await
    }

    async fn validate_inner(
        &self,
        token: &str,
    ) -> Result<OidcClaims, (OidcValidationError, Duration)> {
        let fallback = self.inner.issuers[0].cfg.min_refetch_interval;
        let reject = |r: TokenRejection| (OidcValidationError::InvalidToken(r), fallback);

        let header =
            jsonwebtoken::decode_header(token).map_err(|_| reject(TokenRejection::Undecodable))?;

        let issuer = if self.inner.select_by_iss {
            let iss = unverified_issuer(token).map_err(|_| reject(TokenRejection::Malformed))?;
            iss.and_then(|iss| self.inner.issuers.iter().find(|i| i.issuer_url == iss))
                .ok_or_else(|| reject(TokenRejection::UnknownIssuer))?
        } else {
            &self.inner.issuers[0]
        };

        issuer
            .validate(token, &header)
            .await
            .map_err(|e| (e, issuer.cfg.min_refetch_interval))
    }
}

fn http_client() -> reqwest::Client {
    reqwest::Client::builder()
        .user_agent(concat!("cli-framework-oidc/", env!("CARGO_PKG_VERSION")))
        .build()
        .expect("reqwest client")
}

// ── Main entry point ────────────────────────────────────────────────────────

/// The boxed tower layer returned by [`oidc_validation_layer`],
/// [`oidc_validation_layer_multi`] and [`OidcValidator::layer`].
pub type BoxedOidcLayer = tower::util::BoxCloneSyncServiceLayer<
    cli_framework::axum::Router,
    cli_framework::axum::http::Request<cli_framework::axum::body::Body>,
    cli_framework::axum::response::Response,
    std::convert::Infallible,
>;

/// Build a tower [`Layer`] that validates JWT bearer tokens on every request.
pub fn oidc_validation_layer(cfg: OidcValidationConfig) -> Result<BoxedOidcLayer, OidcConfigError> {
    Ok(OidcValidator::new(cfg)?.layer())
}

/// Build a tower [`Layer`] that accepts bearer tokens from any of several
/// trusted issuers. See [`OidcValidator::new_multi`] for how a token is
/// matched to an issuer.
pub fn oidc_validation_layer_multi(
    cfgs: impl IntoIterator<Item = OidcValidationConfig>,
) -> Result<BoxedOidcLayer, OidcConfigError> {
    Ok(OidcValidator::new_multi(cfgs)?.layer())
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
            match validator.authorize_inner(auth.as_deref()).await {
                Ok(claims) => {
                    req.extensions_mut().insert(claims);
                    inner.call(req).await
                }
                Err((e, retry_after)) => Ok(error_to_response(&e, retry_after)),
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

/// `retry_after` is the `min_refetch_interval` of the issuer whose keys were
/// unavailable; it is only read for [`OidcValidationError::JwksUnavailable`].
fn error_to_response(
    err: &OidcValidationError,
    retry_after: Duration,
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
            .header("retry-after", retry_after.as_secs().to_string())
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
        TokenRejection::UnknownIssuer => "unknown_issuer",
    }
}

// ── JWT error -> TokenRejection ───────────────────────────────────────────────

fn jwt_err_to_rejection(e: &jsonwebtoken::errors::Error) -> TokenRejection {
    use jsonwebtoken::errors::ErrorKind;
    if crate::jwks::is_missing_iss(e) {
        return TokenRejection::InvalidIssuer;
    }
    if crate::jwks::is_missing_aud(e) {
        return TokenRejection::InvalidAudience;
    }
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

fn try_validate_jwt(
    token: &str,
    key: &DecodingKey,
    issuer: &IssuerState,
) -> Result<OidcClaims, TokenRejection> {
    let cfg = &issuer.cfg;
    let issuer_url = issuer.issuer_url.as_str();
    let mut validation = Validation::new(cfg.algorithms[0]);
    validation.algorithms = cfg.algorithms.clone();
    validation.set_issuer(&[issuer_url]);
    validation.set_required_spec_claims(crate::jwks::REQUIRED_SPEC_CLAIMS);
    crate::jwks::apply_audience_policy(&mut validation, &cfg.audience);
    validation.leeway = cfg.clock_skew.as_secs();

    let token_data = jsonwebtoken::decode::<JsonValue>(token, key, &validation)
        .map_err(|e| jwt_err_to_rejection(&e))?;

    let claims = &token_data.claims;

    let sub = claims["sub"]
        .as_str()
        .ok_or(TokenRejection::Malformed)?
        .to_string();
    // `iss` is required and was just checked against `issuer_url`, the
    // normalized configured issuer, so `OidcClaims::iss` names the issuer that
    // validated the token. `jsonwebtoken` also accepts an array `iss` that
    // contains the issuer; RFC 7519 makes `iss` a single string, so refuse it.
    let iss = claims["iss"]
        .as_str()
        .ok_or(TokenRejection::InvalidIssuer)?
        .to_string();
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

    let roles = issuer.roles_path.strings(claims);
    let groups = issuer
        .groups_path
        .as_ref()
        .map(|p| p.strings(claims))
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
        groups,
        raw: claims.clone(),
    })
}
