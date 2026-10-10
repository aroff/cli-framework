//! Host sessions: a server signs a person in through their realm as a
//! confidential client and keeps their tokens in a sealed session cookie, so
//! the browser never holds a token.
//!
//! This is the shape a web host needs when it calls a back end *for* the
//! person (a backend-for-frontend), as opposed to [`crate::browser`], whose
//! layer guards an SPA's own routes:
//!
//! - a confidential client (`client_secret` at the token, refresh and logout
//!   calls), with PKCE, `state` and an ID-token `nonce`;
//! - a session cookie with no `Max-Age` by default, so closing the browser
//!   ends it, plus an idle timeout whose last-activity stamp is rewritten only
//!   after a tenth of the window has passed;
//! - a random 128-bit session id and a caller-supplied **binding** sealed in
//!   the cookie and checked on every read;
//! - access tokens checked to be issued to this client (`azp` == `client_id`)
//!   at sign-in and after every refresh;
//! - routes under a configurable prefix (`{prefix}/login`, `/callback`,
//!   `/logout`, `/session`) and a configurable cookie name (`__Host-session`
//!   by default);
//! - [`HostSessions::resolve`], which the caller runs on its own routes to get
//!   the person's current access token, refreshing it when it is close to
//!   expiry.
//!
//! The cookie is one AES-256-GCM layer over a compact binary record (see
//! `seal.rs`); [`HostSessions::new`] refuses a configuration whose expected
//! cookie would exceed `max_cookie_bytes`, and sign-in refuses a real one that
//! does.

mod handlers;
mod seal;

pub use crate::browser::SessionKey;
use crate::server::{OidcValidationConfig, OidcValidator};
use crate::types::{AudiencePolicy, OidcClaims};
use crate::OidcConfigError;

use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use cli_framework::axum::http::{header, HeaderMap, HeaderValue};
use jsonwebtoken::Algorithm;
use rand::RngCore;
use seal::{Purpose, Sealer, SessionRecord};
use serde::Deserialize;
use serde_json::Value as JsonValue;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::sync::OnceCell;
use zeroize::Zeroizing;

/// The time source, in Unix seconds. [`Clock::system`] outside tests.
#[derive(Clone)]
pub struct Clock(Arc<dyn Fn() -> i64 + Send + Sync>);

impl Clock {
    pub fn system() -> Self {
        Self(Arc::new(|| {
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|d| d.as_secs() as i64)
                .unwrap_or(0)
        }))
    }

    /// A clock read from `f`; for tests that move time.
    pub fn from_fn(f: impl Fn() -> i64 + Send + Sync + 'static) -> Self {
        Self(Arc::new(f))
    }

    fn now(&self) -> i64 {
        (self.0)()
    }
}

/// The confidential client's secret. No `Debug`, zeroized on drop.
#[derive(Clone)]
pub struct ClientSecret(Arc<Zeroizing<String>>);

impl ClientSecret {
    pub fn new(secret: impl Into<String>) -> Self {
        Self(Arc::new(Zeroizing::new(secret.into())))
    }

    fn expose(&self) -> &str {
        self.0.as_str()
    }
}

/// Configuration for [`HostSessions`]. Build with [`HostSessionConfig::new`]
/// and override fields as needed.
#[derive(Clone)]
pub struct HostSessionConfig {
    /// The realm's issuer URL.
    pub issuer_url: String,
    /// The host's client in the realm.
    pub client_id: String,
    /// The client's secret. `None` makes it a public client (PKCE only).
    pub client_secret: Option<ClientSecret>,
    /// The absolute callback URL registered for the client; its origin is the
    /// host's own origin, which the logout route checks `Origin` against.
    pub redirect_uri: String,
    /// Where the realm's end-session endpoint sends the browser after logout.
    pub post_logout_redirect_uri: String,
    /// The 32-byte key the cookies are sealed with.
    pub session_key: SessionKey,
    /// What the session is bound to, sealed in the cookie and checked on read.
    pub binding: String,
    /// The routes' prefix, e.g. `/_host` (no trailing slash).
    pub route_prefix: String,
    /// The session cookie's name. The default `__Host-session` makes the
    /// browser refuse it without `Secure`, `Path=/` and no `Domain`.
    pub cookie_name: String,
    /// The sign-in state cookie's name (lives `sign_in_ttl`).
    pub sign_in_cookie_name: String,
    /// A request more than this long after the last-activity stamp ends the session.
    pub idle_timeout: Duration,
    /// `None` (default): a session cookie with no `Max-Age`. `Some(ttl)`:
    /// `Max-Age` = min(refresh token lifetime, ttl).
    pub session_ttl: Option<Duration>,
    /// Refresh the access token when it expires within this window.
    pub refresh_skew: Duration,
    /// Scopes requested at sign-in; `openid` is always sent.
    pub scopes: Vec<String>,
    /// Accepted signature algorithms for the ID and access tokens.
    pub algorithms: Vec<Algorithm>,
    /// Audience policy for the access token (the ID token's audience is always the client).
    pub access_audience: AudiencePolicy,
    /// `true` (default): the access token's `azp` (authorized party) must be
    /// `client_id`, i.e. the realm issued it to this host. Sign-in refuses an
    /// access token with another or no `azp`, and a refresh that returns one
    /// ends the session as [`EndReason::RefreshRefused`]. This is the check
    /// that ties the access token to the host: its `aud` names the APIs it is
    /// meant for, which is usually not the host itself, so with this on an
    /// `Unchecked` `access_audience` logs no warning. Set `false` only for a
    /// provider whose access tokens carry no `azp`, and set `access_audience`
    /// then.
    pub require_access_azp: bool,
    /// JWKS URI override (default: from discovery).
    pub jwks_uri: Option<String>,
    pub jwks_ttl: Duration,
    pub clock_skew: Duration,
    /// Timeout of each call to the realm.
    pub http_timeout: Duration,
    /// The largest `name=value` the session cookie may be.
    pub max_cookie_bytes: usize,
    /// The access and refresh token sizes the startup size check assumes.
    pub expected_token_bytes: (usize, usize),
    /// Access-token claims that `{prefix}/session` returns besides `sub`.
    pub session_claims: Vec<String>,
    /// How long a sign-in may take between `/login` and `/callback`.
    pub sign_in_ttl: Duration,
    pub clock: Clock,
}

impl HostSessionConfig {
    pub fn new(
        issuer_url: impl Into<String>,
        client_id: impl Into<String>,
        redirect_uri: impl Into<String>,
        session_key: SessionKey,
        binding: impl Into<String>,
    ) -> Self {
        let redirect_uri = redirect_uri.into();
        let post_logout = url::Url::parse(&redirect_uri)
            .map(|u| format!("{}/", u.origin().ascii_serialization()))
            .unwrap_or_default();
        Self {
            issuer_url: issuer_url.into(),
            client_id: client_id.into(),
            client_secret: None,
            redirect_uri,
            post_logout_redirect_uri: post_logout,
            session_key,
            binding: binding.into(),
            route_prefix: "/_host".into(),
            cookie_name: "__Host-session".into(),
            sign_in_cookie_name: "__Host-sign-in".into(),
            idle_timeout: Duration::from_secs(30 * 60),
            session_ttl: None,
            refresh_skew: Duration::from_secs(60),
            scopes: vec!["openid".into()],
            algorithms: vec![Algorithm::RS256],
            access_audience: AudiencePolicy::Unchecked,
            require_access_azp: true,
            jwks_uri: None,
            jwks_ttl: Duration::from_secs(300),
            clock_skew: Duration::from_secs(60),
            http_timeout: Duration::from_secs(10),
            max_cookie_bytes: 4096,
            expected_token_bytes: (2048, 1024),
            session_claims: vec!["name".into(), "preferred_username".into(), "email".into()],
            sign_in_ttl: Duration::from_secs(600),
            clock: Clock::system(),
        }
    }
}

/// Why [`HostSessions::resolve`] found no usable session.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EndReason {
    /// No session cookie.
    NoSession,
    /// The cookie isn't one this key sealed, or is malformed.
    Invalid,
    /// The cookie is bound to something else.
    BindingMismatch,
    /// Idle for longer than `idle_timeout`.
    Idle,
    /// The refresh token (or `session_ttl`) has expired.
    Expired,
    /// The realm refused the refresh token.
    RefreshRefused,
    /// The access token is expired and the realm couldn't be reached to
    /// refresh it. The cookie is kept: this is not the end of the session.
    Unavailable,
}

/// The outcome of [`HostSessions::resolve`].
pub enum Resolution {
    Active(ActiveSession),
    Ended {
        reason: EndReason,
        /// A `Set-Cookie` that deletes the session cookie, when there is one to delete.
        clear_cookie: Option<HeaderValue>,
    },
}

/// A live session. Add [`set_cookie`](Self::set_cookie) to the response when present.
pub struct ActiveSession {
    sid: String,
    access_token: Zeroizing<String>,
    claims: JsonValue,
    refresh_exp: i64,
    idle_exp: i64,
    set_cookie: Option<HeaderValue>,
}

impl ActiveSession {
    /// The person's current access token. Never log it.
    pub fn access_token(&self) -> &str {
        self.access_token.as_str()
    }
    /// The session id (32 hex characters), stable for the session's life.
    pub fn sid(&self) -> &str {
        &self.sid
    }
    /// The access token's claims (verified when the token was obtained).
    pub fn claims(&self) -> &JsonValue {
        &self.claims
    }
    /// Unix seconds when the session ends at the latest.
    pub fn expires_at(&self) -> i64 {
        self.refresh_exp
    }
    /// Unix seconds when the session ends if no request comes.
    pub fn idle_expires_at(&self) -> i64 {
        self.idle_exp
    }
    /// A rewritten session cookie (refreshed tokens or a new stamp), to send back.
    pub fn set_cookie(&self) -> Option<&HeaderValue> {
        self.set_cookie.as_ref()
    }
}

/// The host's session manager. Cheap to clone.
#[derive(Clone)]
pub struct HostSessions {
    inner: Arc<Inner>,
}

pub(crate) struct Inner {
    cfg: HostSessionConfig,
    issuer: String,
    origin: String,
    sealer: Sealer,
    id_tokens: OidcValidator,
    access_tokens: OidcValidator,
    http: reqwest::Client,
    discovery: OnceCell<Discovery>,
}

#[derive(Deserialize)]
pub(crate) struct Discovery {
    issuer: String,
    authorization_endpoint: String,
    token_endpoint: String,
    end_session_endpoint: Option<String>,
}

/// A token endpoint response.
#[derive(Deserialize)]
pub(crate) struct TokenResponse {
    access_token: String,
    refresh_token: Option<String>,
    id_token: Option<String>,
    refresh_expires_in: Option<i64>,
}

/// Why a token endpoint call failed.
pub(crate) enum TokenCallError {
    /// The realm answered `400 invalid_grant` (or another OAuth error).
    Refused(String),
    /// Network, timeout, 5xx or an unreadable answer.
    Unavailable(String),
}

impl HostSessions {
    /// Validates the configuration; makes no network call.
    pub fn new(cfg: HostSessionConfig) -> Result<Self, OidcConfigError> {
        let issuer = crate::normalize_issuer(&cfg.issuer_url)?;
        if let Some(uri) = &cfg.jwks_uri {
            crate::validate_jwks_uri(uri)?;
        }
        let bad = |what: &str| OidcConfigError::InvalidFlow(what.to_string());
        let redirect = url::Url::parse(&cfg.redirect_uri).map_err(|_| bad("redirect_uri"))?;
        if !(redirect.scheme() == "https" || is_loopback(&redirect)) {
            return Err(bad("redirect_uri must be https (or loopback http)"));
        }
        let origin = redirect.origin().ascii_serialization();
        let prefix = &cfg.route_prefix;
        if !prefix.starts_with('/') || prefix.ends_with('/') || prefix.len() < 2 {
            return Err(bad("route_prefix must start with / and not end with /"));
        }
        if redirect.path() != format!("{prefix}/callback") {
            return Err(bad("redirect_uri's path must be {route_prefix}/callback"));
        }
        if cfg.client_id.is_empty() {
            return Err(OidcConfigError::MissingField("client_id"));
        }
        if cfg.binding.is_empty() {
            return Err(OidcConfigError::MissingField("binding"));
        }
        if cfg.algorithms.is_empty() {
            return Err(OidcConfigError::EmptyAlgorithms);
        }
        if cfg.idle_timeout.as_secs() < 60 {
            return Err(bad("idle_timeout must be at least a minute"));
        }
        for name in [&cfg.cookie_name, &cfg.sign_in_cookie_name] {
            if name.is_empty() || !name.bytes().all(is_cookie_name_byte) {
                return Err(bad("cookie names must be RFC 6265 tokens"));
            }
        }
        let expected = cfg.cookie_name.len()
            + 1
            + seal::sealed_len(
                seal::SESSION_FIXED_LEN
                    + cfg.binding.len()
                    + cfg.expected_token_bytes.0 * 3 / 4
                    + cfg.expected_token_bytes.1 * 3 / 4
                    + 16,
            );
        if expected > cfg.max_cookie_bytes {
            return Err(OidcConfigError::InvalidFlow(format!(
                "a session cookie with the expected token sizes would be {expected} bytes, \
                 over max_cookie_bytes {}",
                cfg.max_cookie_bytes
            )));
        }

        let validator_config = |audience: AudiencePolicy| {
            let mut v = OidcValidationConfig::new(issuer.clone(), audience);
            v.algorithms = cfg.algorithms.clone();
            v.jwks_uri = cfg.jwks_uri.clone();
            v.jwks_ttl = cfg.jwks_ttl;
            v.clock_skew = cfg.clock_skew;
            v
        };
        let id_tokens = OidcValidator::new(validator_config(AudiencePolicy::Require(
            cfg.client_id.clone(),
        )))?;
        // With the `azp` check on, an `Unchecked` audience is not "no binding":
        // the token is still tied to this client, so the validator's WARN
        // would be untrue here.
        let access_config = validator_config(cfg.access_audience.clone());
        let access_tokens = if cfg.require_access_azp {
            OidcValidator::new_without_audience_warning(access_config)?
        } else {
            OidcValidator::new(access_config)?
        };
        let http = reqwest::Client::builder()
            .user_agent(concat!("cli-framework-oidc/", env!("CARGO_PKG_VERSION")))
            .redirect(reqwest::redirect::Policy::none())
            .timeout(cfg.http_timeout)
            .connect_timeout(cfg.http_timeout.min(Duration::from_secs(5)))
            .build()
            .map_err(|e| OidcConfigError::InvalidFlow(format!("http client: {e}")))?;
        Ok(Self {
            inner: Arc::new(Inner {
                sealer: Sealer::new(cfg.session_key.as_bytes()),
                cfg,
                issuer,
                origin,
                id_tokens,
                access_tokens,
                http,
                discovery: OnceCell::new(),
            }),
        })
    }

    /// The routes `{prefix}/login` (GET), `/callback` (GET), `/logout` (POST)
    /// and `/session` (GET).
    pub fn router(&self) -> cli_framework::axum::Router {
        handlers::router(Arc::clone(&self.inner))
    }

    /// The host's own origin (`scheme://host[:port]`), from `redirect_uri`.
    pub fn origin(&self) -> &str {
        &self.inner.origin
    }

    /// Reads the session cookie from `headers` and decides whether the
    /// session is live: binding, idle, expiry, and a refresh when the access
    /// token is within `refresh_skew` of expiry.
    pub async fn resolve(&self, headers: &HeaderMap) -> Resolution {
        self.inner.resolve(headers).await
    }

    /// A `Set-Cookie` value that deletes the session cookie.
    pub fn clear_cookie(&self) -> HeaderValue {
        self.inner.clear_cookie()
    }
}

impl Inner {
    fn now(&self) -> i64 {
        self.cfg.clock.now()
    }

    pub(crate) async fn resolve(&self, headers: &HeaderMap) -> Resolution {
        let Some(value) = read_cookie(headers, &self.cfg.cookie_name) else {
            return Resolution::Ended {
                reason: EndReason::NoSession,
                clear_cookie: None,
            };
        };
        let end = |reason| Resolution::Ended {
            reason,
            clear_cookie: Some(self.clear_cookie()),
        };
        let Some(mut rec) = self
            .sealer
            .open(Purpose::Session, &value)
            .and_then(|b| SessionRecord::decode(&b))
        else {
            return end(EndReason::Invalid);
        };
        if !constant_time_eq(rec.binding.as_bytes(), self.cfg.binding.as_bytes()) {
            return end(EndReason::BindingMismatch);
        }
        let now = self.now();
        let idle = self.cfg.idle_timeout.as_secs() as i64;
        if now - rec.last_activity > idle {
            return end(EndReason::Idle);
        }
        if now >= rec.refresh_exp {
            return end(EndReason::Expired);
        }

        let mut rewrite = now - rec.last_activity > idle / 10;
        if now + self.cfg.refresh_skew.as_secs() as i64 >= rec.access_exp {
            match self.refresh(&rec.refresh_token).await {
                Ok(tokens) => match self.verified_access(&tokens.access_token).await {
                    Ok(access) => {
                        rec.refresh_exp = self.refresh_exp(&tokens, now, rec.refresh_exp);
                        rec.access_exp = access.exp;
                        rec.access_token = Zeroizing::new(tokens.access_token);
                        if let Some(rt) = tokens.refresh_token {
                            rec.refresh_token = Zeroizing::new(rt);
                        }
                        rewrite = true;
                    }
                    Err(e) => {
                        tracing::warn!("host-session: refreshed access token refused: {e}");
                        return end(EndReason::RefreshRefused);
                    }
                },
                Err(TokenCallError::Refused(e)) => {
                    tracing::info!("host-session: refresh refused by the realm: {e}");
                    return end(EndReason::RefreshRefused);
                }
                Err(TokenCallError::Unavailable(e)) => {
                    tracing::warn!("host-session: refresh failed: {e}");
                    if now >= rec.access_exp {
                        return Resolution::Ended {
                            reason: EndReason::Unavailable,
                            clear_cookie: None,
                        };
                    }
                }
            }
        }

        let set_cookie = if rewrite {
            rec.last_activity = now;
            match self.session_cookie(&rec) {
                Ok(c) => Some(c),
                Err(size) => {
                    tracing::warn!(size, "host-session: refreshed cookie is over the limit");
                    return end(EndReason::Invalid);
                }
            }
        } else {
            None
        };
        Resolution::Active(ActiveSession {
            sid: hex(&rec.sid),
            claims: unverified_payload(&rec.access_token).unwrap_or(JsonValue::Null),
            access_token: rec.access_token,
            refresh_exp: rec.refresh_exp,
            idle_exp: rec.last_activity + idle,
            set_cookie,
        })
    }

    /// The new refresh expiry: the realm's `refresh_expires_in` when it gives
    /// one, else the refresh token's own `exp`, else the previous value.
    fn refresh_exp(&self, tokens: &TokenResponse, now: i64, previous: i64) -> i64 {
        match tokens.refresh_expires_in {
            Some(secs) if secs > 0 => now + secs,
            _ => tokens
                .refresh_token
                .as_deref()
                .and_then(unverified_payload)
                .and_then(|p| p["exp"].as_i64())
                .unwrap_or(previous),
        }
    }

    /// Verifies an access token against the realm (signature, `iss`, `exp`,
    /// `access_audience`) and, with `require_access_azp`, that it was issued
    /// to this client.
    pub(crate) async fn verified_access(&self, token: &str) -> Result<OidcClaims, String> {
        let claims = self
            .access_tokens
            .validate(token)
            .await
            .map_err(|e| e.to_string())?;
        if self.cfg.require_access_azp {
            match claims.raw.get("azp").and_then(JsonValue::as_str) {
                Some(azp) if azp == self.cfg.client_id => {}
                Some(azp) => return Err(format!("issued to another client (azp {azp:?})")),
                None => {
                    return Err("no azp claim; cannot tell which client it was issued to".into())
                }
            }
        }
        Ok(claims)
    }

    /// A new session record for freshly obtained tokens.
    fn new_record(&self, tokens: TokenResponse, access_exp: i64) -> Result<SessionRecord, String> {
        let now = self.now();
        let refresh_exp = self.refresh_exp(&tokens, now, 0);
        let refresh_token = tokens
            .refresh_token
            .ok_or("the realm returned no refresh token")?;
        if refresh_exp <= now {
            return Err("the realm returned no refresh token lifetime".into());
        }
        let mut sid = [0u8; 16];
        rand::rngs::OsRng.fill_bytes(&mut sid);
        Ok(SessionRecord {
            sid,
            last_activity: now,
            access_exp,
            refresh_exp,
            binding: self.cfg.binding.clone(),
            access_token: Zeroizing::new(tokens.access_token),
            refresh_token: Zeroizing::new(refresh_token),
        })
    }

    /// The `Set-Cookie` for `rec`, or its size when over `max_cookie_bytes`.
    fn session_cookie(&self, rec: &SessionRecord) -> Result<HeaderValue, usize> {
        let value = self.sealer.seal(Purpose::Session, &rec.encode());
        let size = self.cfg.cookie_name.len() + 1 + value.len();
        if size > self.cfg.max_cookie_bytes {
            return Err(size);
        }
        let max_age = self
            .cfg
            .session_ttl
            .map(|ttl| {
                let left = (rec.refresh_exp - self.now()).max(0) as u64;
                format!("; Max-Age={}", left.min(ttl.as_secs()))
            })
            .unwrap_or_default();
        Ok(HeaderValue::from_str(&format!(
            "{}={value}; Path=/; Secure; HttpOnly; SameSite=Lax{max_age}",
            self.cfg.cookie_name
        ))
        .expect("base64url and fixed attributes are a valid header value"))
    }

    pub(crate) fn clear_cookie(&self) -> HeaderValue {
        HeaderValue::from_str(&format!(
            "{}=; Path=/; Secure; HttpOnly; SameSite=Lax; Max-Age=0",
            self.cfg.cookie_name
        ))
        .expect("valid header value")
    }

    async fn discovery(&self) -> Result<&Discovery, String> {
        self.discovery
            .get_or_try_init(|| async {
                let url = format!("{}/.well-known/openid-configuration", self.issuer);
                let resp = self
                    .http
                    .get(&url)
                    .send()
                    .await
                    .map_err(|e| format!("discovery: {e}"))?;
                if !resp.status().is_success() {
                    return Err(format!("discovery: HTTP {}", resp.status()));
                }
                let doc: Discovery = resp.json().await.map_err(|e| format!("discovery: {e}"))?;
                let found = crate::normalize_issuer(&doc.issuer).map_err(|e| e.to_string())?;
                if found != self.issuer {
                    return Err(format!("discovery issuer mismatch: {found}"));
                }
                Ok(doc)
            })
            .await
    }

    /// A call to the token endpoint with the client's credentials added.
    async fn token_call(&self, form: &[(&str, &str)]) -> Result<TokenResponse, TokenCallError> {
        let endpoint = self
            .discovery()
            .await
            .map_err(TokenCallError::Unavailable)?
            .token_endpoint
            .clone();
        let mut body: Vec<(&str, &str)> = form.to_vec();
        body.push(("client_id", &self.cfg.client_id));
        if let Some(secret) = &self.cfg.client_secret {
            body.push(("client_secret", secret.expose()));
        }
        let resp = self
            .http
            .post(&endpoint)
            .form(&body)
            .send()
            .await
            .map_err(|e| TokenCallError::Unavailable(format!("token endpoint: {e}")))?;
        let status = resp.status();
        if status.is_success() {
            return resp
                .json()
                .await
                .map_err(|e| TokenCallError::Unavailable(format!("token response: {e}")));
        }
        let error = resp
            .json::<JsonValue>()
            .await
            .ok()
            .and_then(|v| v["error"].as_str().map(String::from));
        match (status.as_u16(), error) {
            (400 | 401, Some(e)) => Err(TokenCallError::Refused(e)),
            (code, _) => Err(TokenCallError::Unavailable(format!(
                "token endpoint: HTTP {code}"
            ))),
        }
    }

    async fn refresh(&self, refresh_token: &str) -> Result<TokenResponse, TokenCallError> {
        self.token_call(&[
            ("grant_type", "refresh_token"),
            ("refresh_token", refresh_token),
        ])
        .await
    }
}

fn is_loopback(u: &url::Url) -> bool {
    u.scheme() == "http" && matches!(u.host_str(), Some("127.0.0.1" | "localhost" | "[::1]"))
}

fn is_cookie_name_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b"!#$%&'*+-.^_`|~".contains(&b)
}

/// The value of cookie `name` in the request's `Cookie` headers.
pub(crate) fn read_cookie(headers: &HeaderMap, name: &str) -> Option<String> {
    headers
        .get_all(header::COOKIE)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .flat_map(|v| v.split(';'))
        .filter_map(|pair| pair.trim().split_once('='))
        .find(|(k, _)| *k == name)
        .map(|(_, v)| v.to_string())
}

/// A JWT's payload without verifying it: only for tokens this server verified
/// when it got them, or for reading a refresh token's `exp`.
fn unverified_payload(token: &str) -> Option<JsonValue> {
    let payload = token.split('.').nth(1)?;
    serde_json::from_slice(&URL_SAFE_NO_PAD.decode(payload).ok()?).ok()
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

pub(crate) fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

pub(crate) fn random_token() -> String {
    let mut bytes = [0u8; 32];
    rand::rngs::OsRng.fill_bytes(&mut bytes);
    URL_SAFE_NO_PAD.encode(bytes)
}
