//! [`PolicyClient`]: fetches `GET /v1/policy/{app}` and applies spec 021's
//! failure-mapping table.
//!
//! This is the single most safety-critical piece of this slice. The
//! distinction that matters is not the HTTP status code by itself but what
//! it implies about the *token*: a token that is expired, malformed, or
//! otherwise rejected reads identically — from this client's point of view —
//! to a token belonging to a user whose access was just revoked. Falling
//! back to a cached policy on `401` would defeat the revocation guarantee
//! this contract exists to make. See [`PolicyOutcome::Denied`] and the
//! `classify` function below.

use super::cache::{now_epoch_secs, PolicyCache, PolicyCacheEntry};
use crate::auth::AuthenticatedHttpClient;
use crate::config::{ConfigError, Policy, StaleAction};
use reqwest::header::{ETAG, IF_NONE_MATCH};
use reqwest::StatusCode;
use std::sync::Arc;

/// The result of a single policy fetch attempt, once the failure-mapping
/// table has been applied. Every variant here is something a caller may
/// legitimately act on — none of them are "the request failed," which is
/// instead [`PolicyClientError`].
#[derive(Debug, Clone, PartialEq)]
pub enum PolicyOutcome {
    /// `200 OK`, or `304 Not Modified` re-serving the cached body — either
    /// way, this is the server's current word on the policy. The cache's
    /// fetch time is refreshed in both cases (spec 021: "Both a 200 and a
    /// 304 refresh the fetch time").
    Fresh(Policy),
    /// The server was unreachable (network failure or 5xx) and a cached
    /// policy was used instead, per the cached policy's own
    /// `max_cache_age_secs` / `stale_action`. `stale` is `true` when the
    /// cache has aged past `max_cache_age_secs` and `stale_action` is
    /// [`StaleAction::Warn`] (a [`StaleAction::Refuse`] policy instead
    /// surfaces [`PolicyClientError::StaleCacheRefused`] and never reaches
    /// this variant).
    FromCache { policy: Policy, stale: bool },
    /// `404 Not Found`: this identity is not managed for this application.
    /// The cache has already been cleared by the time this is returned.
    Unmanaged,
    /// `401` (where a `TokenProvider` retry also failed to produce a working
    /// token) or `403`. These two cases are **deliberately** collapsed into
    /// one observable outcome — see the module docs — and never read the
    /// cache.
    Denied,
}

/// Failures [`PolicyClient::fetch`] cannot resolve into a [`PolicyOutcome`]:
/// the server could not be reached (or returned 5xx) and there is no usable
/// cache to fall back to, or a usable cache exists but is stale and its own
/// `stale_action` is [`StaleAction::Refuse`].
#[derive(thiserror::Error, Debug)]
#[non_exhaustive]
pub enum PolicyClientError {
    #[error("policy for '{app}' is unreachable and no usable cached policy exists: {message}")]
    Unreachable { app: String, message: String },

    #[error(
        "cached policy for '{app}' is older than its own max_cache_age_secs and stale_action=refuse"
    )]
    StaleCacheRefused { app: String },

    #[error("cached policy for '{app}' could not be parsed: {message}")]
    CacheCorrupt { app: String, message: String },

    #[error("policy response body for '{app}' could not be parsed: {message}")]
    InvalidResponse { app: String, message: String },

    #[error("policy cache storage error: {0}")]
    Cache(#[from] ConfigError),

    /// A `4xx` response that is none of `401`/`403`/`404` and not a `5xx` —
    /// e.g. `400 Bad Request`, `409 Conflict`, `422 Unprocessable Entity`, or
    /// `429 Too Many Requests`. Per spec 021's failure-mapping table, cache
    /// fallback is reserved for "server error or network failure... no HTTP
    /// response at all, or 5xx" — nothing else. A malformed request or a rate
    /// limit is a hard, non-cache-eligible error: it must not silently keep
    /// serving a stale cached policy forever with no visible signal anything
    /// is wrong. See [`HttpFailureClass::ClientError`].
    #[error("policy request for '{app}' was rejected with client error {status}: {message}")]
    ClientError {
        app: String,
        status: u16,
        message: String,
    },

    /// A [`Policy`] was fetched (or read from cache) successfully, but
    /// folding it into an application's typed config value failed — either
    /// the current value could not be serialized/read, or the folded result
    /// did not deserialize back into the target type. Produced by
    /// `crate::config::managed::refresh_managed_config` and
    /// `AppBuilder::build_with_config`'s policy-aware path, never by
    /// [`PolicyClient::fetch`] itself.
    #[error("policy could not be folded into the config for '{app}': {message}")]
    FoldFailed { app: String, message: String },
}

/// Fetches and caches the [`Policy`] for one application, applying the
/// exact spec 021 failure-mapping table.
pub struct PolicyClient {
    http: Arc<AuthenticatedHttpClient>,
    cache: PolicyCache,
    base_url: String,
    app: String,
}

impl PolicyClient {
    pub fn new(
        http: Arc<AuthenticatedHttpClient>,
        cache: PolicyCache,
        base_url: impl Into<String>,
        app: impl Into<String>,
    ) -> Self {
        Self {
            http,
            cache,
            base_url: base_url.into(),
            app: app.into(),
        }
    }

    fn url(&self) -> String {
        format!(
            "{}/v1/policy/{}",
            self.base_url.trim_end_matches('/'),
            self.app
        )
    }

    /// Fetch the current policy, applying ETag revalidation against the
    /// cache and the failure-mapping table on any non-2xx/304 outcome.
    ///
    /// Side effects: on [`PolicyOutcome::Fresh`] the cache is written; on
    /// [`PolicyOutcome::Unmanaged`] the cache is cleared; on
    /// [`PolicyOutcome::Denied`] or a fallback to cache, the cache is left
    /// untouched.
    ///
    /// A cache-read failure (corrupt bytes, an unreadable backend) degrades
    /// to "as if no cache existed" rather than aborting the fetch — see
    /// `PolicyCacheEntry`'s own docs. Only [`Self::fall_back_to_cache`]
    /// actually *needs* a readable cache (to serve a 5xx/network failure);
    /// a fresh `200`/`304`/`401`/`403`/`404` outcome never depends on it, so a
    /// single corrupted cache file must not permanently block every future
    /// fetch from ever reaching the network again.
    pub async fn fetch(&self) -> Result<PolicyOutcome, PolicyClientError> {
        let cached = self.cache.read().unwrap_or_else(|e| {
            tracing::warn!(
                "policy cache for '{}' unreadable, treating as absent: {e}",
                self.app
            );
            None
        });
        self.fetch_with_cached(cached).await
    }

    /// The currently cached policy, performing **no network request** —
    /// what a read-only diagnostic (the built-in `config profile`/`config
    /// show` commands, spec 021's "Command surface") reads instead of
    /// forcing a live fetch. Only [`Self::fetch`] / [`Self::background_refresh`]
    /// (and the built-in `config refresh` command) ever touch the network.
    ///
    /// Returns `Ok(None)` when nothing has been cached yet — the caller's cue
    /// to report "unmanaged" rather than treat this as an error.
    pub fn cached_policy(&self) -> Result<Option<Policy>, PolicyClientError> {
        let Some(entry) = self.cache.read()? else {
            return Ok(None);
        };
        let policy: Policy =
            serde_json::from_value(entry.policy).map_err(|e| PolicyClientError::CacheCorrupt {
                app: self.app.clone(),
                message: e.to_string(),
            })?;
        Ok(Some(policy))
    }

    /// Like [`Self::fetch`], but never surfaces an error and never panics —
    /// for a background refresh interval, where spec 021 requires a failure
    /// to "warn and never terminate a running application." Returns `true`
    /// if the fetch produced a usable outcome (whether or not the policy
    /// actually changed).
    pub async fn background_refresh(&self) -> bool {
        match self.fetch().await {
            Ok(_) => true,
            Err(e) => {
                tracing::warn!("policy background refresh for '{}' failed: {e}", self.app);
                false
            }
        }
    }

    async fn fetch_with_cached(
        &self,
        cached: Option<PolicyCacheEntry>,
    ) -> Result<PolicyOutcome, PolicyClientError> {
        let client = self.http.client().clone();
        let url = self.url();
        let etag = cached.as_ref().and_then(|c| c.etag.clone());

        let build = move || {
            let mut rb = client.get(&url);
            if let Some(ref e) = etag {
                rb = rb.header(IF_NONE_MATCH, e.as_str());
            }
            rb
        };

        match self.http.execute_with_retry(build).await {
            Ok(resp) => self.handle_response(resp, cached).await,
            Err(e) => self.handle_error(&e, cached),
        }
    }

    async fn handle_response(
        &self,
        resp: reqwest::Response,
        cached: Option<PolicyCacheEntry>,
    ) -> Result<PolicyOutcome, PolicyClientError> {
        match resp.status() {
            StatusCode::OK => {
                let new_etag = resp
                    .headers()
                    .get(ETAG)
                    .and_then(|v| v.to_str().ok())
                    .map(String::from);
                // `resp.bytes()` failing (a body read/transport error after
                // headers already arrived successfully) is not portably
                // reproducible with a wiremock-backed response — the
                // analogous, directly testable failure mode (malformed body
                // content) is covered by
                // `fresh_200_with_unparseable_json_body_is_invalid_response`.
                let bytes = resp
                    .bytes()
                    .await
                    .map_err(|e| PolicyClientError::InvalidResponse {
                        app: self.app.clone(),
                        message: e.to_string(),
                    })?;
                let raw: serde_json::Value = serde_json::from_slice(&bytes).map_err(|e| {
                    PolicyClientError::InvalidResponse {
                        app: self.app.clone(),
                        message: e.to_string(),
                    }
                })?;
                let policy: Policy = serde_json::from_value(raw.clone()).map_err(|e| {
                    PolicyClientError::InvalidResponse {
                        app: self.app.clone(),
                        message: e.to_string(),
                    }
                })?;
                self.cache.write(&PolicyCacheEntry {
                    policy: raw,
                    etag: new_etag,
                    fetched_at_epoch_secs: now_epoch_secs(),
                })?;
                Ok(PolicyOutcome::Fresh(policy))
            }
            StatusCode::NOT_MODIFIED => {
                let cached = cached.ok_or_else(|| PolicyClientError::InvalidResponse {
                    app: self.app.clone(),
                    message: "server returned 304 Not Modified with no cached policy on file"
                        .to_string(),
                })?;
                let new_etag = resp
                    .headers()
                    .get(ETAG)
                    .and_then(|v| v.to_str().ok())
                    .map(String::from)
                    .or_else(|| cached.etag.clone());
                let refreshed = PolicyCacheEntry {
                    policy: cached.policy.clone(),
                    etag: new_etag,
                    fetched_at_epoch_secs: now_epoch_secs(),
                };
                self.cache.write(&refreshed)?;
                let policy: Policy = serde_json::from_value(refreshed.policy).map_err(|e| {
                    PolicyClientError::CacheCorrupt {
                        app: self.app.clone(),
                        message: e.to_string(),
                    }
                })?;
                Ok(PolicyOutcome::Fresh(policy))
            }
            other => Err(PolicyClientError::InvalidResponse {
                app: self.app.clone(),
                message: format!("unexpected HTTP status {other}"),
            }),
        }
    }

    fn handle_error(
        &self,
        e: &anyhow::Error,
        cached: Option<PolicyCacheEntry>,
    ) -> Result<PolicyOutcome, PolicyClientError> {
        match classify(e) {
            // §Failure mapping, 401/403: identity, not reachability. Never
            // read the cache — see the module docs.
            HttpFailureClass::Unauthorized | HttpFailureClass::Forbidden => {
                Ok(PolicyOutcome::Denied)
            }
            HttpFailureClass::NotFound => {
                self.cache.clear()?;
                Ok(PolicyOutcome::Unmanaged)
            }
            HttpFailureClass::ServerOrNetwork => self.fall_back_to_cache(e, cached),
            // A 4xx that is none of 401/403/404 — a hard, non-cache-eligible
            // error. Never touches `cached` at all (read or write): a
            // malformed request or a rate limit must not silently keep
            // serving a stale cached policy with no visible signal.
            HttpFailureClass::ClientError(status) => Err(PolicyClientError::ClientError {
                app: self.app.clone(),
                status: status.as_u16(),
                message: e.to_string(),
            }),
        }
    }

    fn fall_back_to_cache(
        &self,
        e: &anyhow::Error,
        cached: Option<PolicyCacheEntry>,
    ) -> Result<PolicyOutcome, PolicyClientError> {
        let Some(entry) = cached else {
            return Err(PolicyClientError::Unreachable {
                app: self.app.clone(),
                message: e.to_string(),
            });
        };
        let policy: Policy = serde_json::from_value(entry.policy.clone()).map_err(|err| {
            PolicyClientError::CacheCorrupt {
                app: self.app.clone(),
                message: err.to_string(),
            }
        })?;
        let age = now_epoch_secs().saturating_sub(entry.fetched_at_epoch_secs);
        if age <= policy.max_cache_age_secs {
            return Ok(PolicyOutcome::FromCache {
                policy,
                stale: false,
            });
        }
        match policy.stale_action {
            StaleAction::Warn => Ok(PolicyOutcome::FromCache {
                policy,
                stale: true,
            }),
            StaleAction::Refuse => Err(PolicyClientError::StaleCacheRefused {
                app: self.app.clone(),
            }),
        }
    }
}

/// The five buckets spec 021's failure-mapping table sorts an HTTP outcome
/// into. Deliberately coarser than the raw status code: `401` folds into the
/// same bucket as `403` (see [`PolicyOutcome::Denied`]) rather than getting
/// its own.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum HttpFailureClass {
    Unauthorized,
    Forbidden,
    NotFound,
    /// No HTTP response at all (network/connect/timeout/DNS failure), or a
    /// `5xx` — spec 021's exact, sole definition of "cache fallback".
    ServerOrNetwork,
    /// A `4xx` that is none of `401`/`403`/`404` and is not a `5xx` (`400`,
    /// `409`, `422`, `429`, ...). Spec 021's failure-mapping table only ever
    /// names cache fallback for "server error or network failure" — nothing
    /// else — so this is a hard error, never cache-eligible. Carries the
    /// status for the error message.
    ClientError(StatusCode),
}

/// Classify an error returned by [`AuthenticatedHttpClient::execute_with_retry`].
///
/// `AuthenticatedHttpClient` already performs the spec's "attempt token
/// acquisition once via TokenProvider" step internally (invalidate + one
/// retry on `401`) — see `crate::auth::AuthenticatedHttpClient`. By the time
/// an error reaches here, one of two things happened: the retried request
/// *also* came back `401` (surfaces as a `reqwest::Error` with
/// `status() == 401`), or the token re-acquisition itself failed (surfaces
/// as a `crate::auth::AuthError`, which does not carry an HTTP status at
/// all). Spec 021 requires both to be treated identically to a plain `403` —
/// so both map to [`HttpFailureClass::Unauthorized`], which the caller then
/// handles exactly like [`HttpFailureClass::Forbidden`].
fn classify(e: &anyhow::Error) -> HttpFailureClass {
    if e.downcast_ref::<crate::auth::AuthError>().is_some() {
        return HttpFailureClass::Unauthorized;
    }
    if let Some(re) = e.downcast_ref::<reqwest::Error>() {
        if let Some(status) = re.status() {
            return match status {
                StatusCode::UNAUTHORIZED => HttpFailureClass::Unauthorized,
                StatusCode::FORBIDDEN => HttpFailureClass::Forbidden,
                StatusCode::NOT_FOUND => HttpFailureClass::NotFound,
                _ if status.is_server_error() => HttpFailureClass::ServerOrNetwork,
                _ => HttpFailureClass::ClientError(status),
            };
        }
        // `re.status()` is `None` for every reqwest error this crate can
        // ever observe here (connect/timeout/DNS failures) — handled by the
        // fallthrough below, exercised by
        // `network_failure_with_no_cache_surfaces_as_unreachable`.
    }
    // Neither an `AuthError` nor a downcastable `reqwest::Error` at all: not
    // reachable through the only caller of this function
    // (`AuthenticatedHttpClient::execute_with_retry`, which never produces a
    // third error type), so this fallthrough exists for defensive
    // completeness rather than a case any test constructs.
    HttpFailureClass::ServerOrNetwork
}
