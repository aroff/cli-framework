//! OIDC client: `OidcClient` implementing `cli_framework::auth::TokenProvider`.

use crate::OidcConfigError;
use async_trait::async_trait;
use cli_framework::secrets::{EnvFileSecretStore, SecretKey, SecretStore};
use secrecy::SecretString;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, SystemTime};

mod cache;
use cache::{cache_secret_key, read_cache, write_cache, CacheEntry};

// ── Supporting types ────────────────────────────────────────────────────────

/// Which interactive or automated flow to use.
pub enum OidcFlow {
    DeviceCode,
    AuthCodePkce {
        redirect: RedirectConfig,
    },
    ClientCredentials {
        client_secret: SecretString,
        token_auth: TokenAuthMethod,
    },
}

impl OidcFlow {
    /// Pick an interactive flow based on the runtime environment:
    /// **Auth Code + PKCE** when a local GUI/browser is available, **Device Code**
    /// when running over SSH or on a headless box (no browser to open locally).
    ///
    /// Use this when an app should "just log the user in" without the developer
    /// hard-coding which interactive flow fits the user's environment.
    pub fn auto_interactive() -> OidcFlow {
        let remote =
            std::env::var_os("SSH_CONNECTION").is_some() || std::env::var_os("SSH_TTY").is_some();
        let gui = if cfg!(any(target_os = "macos", target_os = "windows")) {
            true
        } else {
            std::env::var_os("DISPLAY").is_some() || std::env::var_os("WAYLAND_DISPLAY").is_some()
        };
        pick_interactive_flow(remote, gui)
    }
}

/// Pure decision for [`OidcFlow::auto_interactive`] — separated for testability.
fn pick_interactive_flow(remote_session: bool, gui_available: bool) -> OidcFlow {
    if !remote_session && gui_available {
        OidcFlow::AuthCodePkce {
            redirect: RedirectConfig::default(),
        }
    } else {
        OidcFlow::DeviceCode
    }
}

#[derive(Clone, Debug)]
pub struct RedirectConfig {
    pub port: RedirectPort,
}

impl Default for RedirectConfig {
    fn default() -> Self {
        Self {
            port: RedirectPort::Fixed(8765),
        }
    }
}

#[derive(Clone, Debug)]
pub enum RedirectPort {
    Fixed(u16),
    Ephemeral,
}

#[derive(Clone, Copy, Debug, Default)]
pub enum TokenAuthMethod {
    #[default]
    Post,
    Basic,
}

// ── OidcClient ──────────────────────────────────────────────────────────────

struct DiscoveryDoc {
    token_endpoint: String,
    device_authorization_endpoint: Option<String>,
    authorization_endpoint: Option<String>,
}

pub struct OidcClient {
    issuer_url: String,
    client_id: String,
    flow: OidcFlow,
    scopes: Option<Vec<String>>,
    cache_dir: PathBuf,
    reporter: Arc<dyn cli_framework::auth::AuthFlowReporter>,
    refresh_skew: Duration,
    discovery: tokio::sync::OnceCell<DiscoveryDoc>,
    http: reqwest::Client,
    /// Where the token cache is stored. Defaults to an
    /// [`EnvFileSecretStore`] rooted at `cache_dir` — this reproduces the
    /// crate's historical on-disk `oidc-token.json` behavior exactly (see
    /// `client/cache.rs` docs). Inject a different backend (e.g.
    /// OpenBao-backed) via [`OidcClientBuilder::secret_store`] to store
    /// cached tokens somewhere other than a local file.
    secret_store: Arc<dyn SecretStore>,
    cache_secret_key: SecretKey,
}

impl OidcClient {
    /// The on-disk directory holding this client's token cache (`oidc-token.json`).
    /// Either the explicit `cache_dir` or the app-name-derived default.
    pub fn cache_dir(&self) -> &std::path::Path {
        &self.cache_dir
    }

    /// The normalized issuer URL this client authenticates against.
    pub fn issuer_url(&self) -> &str {
        &self.issuer_url
    }

    /// The OAuth client id.
    pub fn client_id(&self) -> &str {
        &self.client_id
    }

    /// The configured grant flow.
    pub fn flow(&self) -> &OidcFlow {
        &self.flow
    }

    pub fn builder() -> OidcClientBuilder {
        OidcClientBuilder::new()
    }

    fn cache_key(&self) -> String {
        use sha2::{Digest, Sha256};
        let mut scopes = self.effective_scopes();
        scopes.sort();
        scopes.dedup();
        let flow_kind = match &self.flow {
            OidcFlow::DeviceCode => "device_code",
            OidcFlow::AuthCodePkce { .. } => "auth_code_pkce",
            OidcFlow::ClientCredentials { .. } => "client_credentials",
        };
        let canonical = format!(
            "{}\n{}\n{}\n{}",
            self.issuer_url,
            self.client_id,
            flow_kind,
            scopes.join(" ")
        );
        let hash = Sha256::digest(canonical.as_bytes());
        hash.iter().map(|b| format!("{:02x}", b)).collect()
    }

    fn effective_scopes(&self) -> Vec<String> {
        if let Some(ref s) = self.scopes {
            return s.clone();
        }
        match &self.flow {
            OidcFlow::DeviceCode | OidcFlow::AuthCodePkce { .. } => {
                vec!["openid".to_string()]
            }
            OidcFlow::ClientCredentials { .. } => vec![],
        }
    }

    async fn get_discovery(&self) -> Result<&DiscoveryDoc, cli_framework::auth::AuthError> {
        self.discovery
            .get_or_try_init(|| fetch_discovery(&self.issuer_url, &self.http))
            .await
    }

    /// Read the current cache entry for `key` (this client's `cache_key()`)
    /// out of the injected `SecretStore`, decomposed into the three fields
    /// callers below need. A read-only lookup — never takes the cache lock.
    async fn read_entry(&self, key: &str) -> (Option<String>, Option<String>, Option<SystemTime>) {
        let cache = read_cache(self.secret_store.as_ref(), &self.cache_secret_key).await;
        match cache.entries.get(key).cloned() {
            Some(entry) => {
                let exp = entry.expires_at.as_deref().and_then(cache::parse_rfc3339);
                (entry.access_token, entry.refresh_token, exp)
            }
            None => (None, None, None),
        }
    }

    async fn get_or_refresh_token(
        &self,
    ) -> Result<cli_framework::auth::AccessToken, cli_framework::auth::AuthError> {
        let key = self.cache_key();
        let refresh_skew = self.refresh_skew;

        let (access_token, refresh_token, expires_at) = self.read_entry(&key).await;

        // Check if access token is still fresh
        if let Some(ref at) = access_token {
            let is_fresh = expires_at.is_none_or(|exp| {
                SystemTime::now()
                    .checked_add(refresh_skew)
                    .is_some_and(|t| t < exp)
            });
            if is_fresh {
                return Ok(cli_framework::auth::AccessToken::new(
                    at.clone(),
                    expires_at,
                ));
            }
        }

        // Try refresh if we have a refresh token
        if let Some(rt) = refresh_token {
            let discovery = self.get_discovery().await?;
            match self.do_refresh(discovery, &rt).await {
                Ok(token) => return Ok(token),
                Err(e) => {
                    tracing::warn!("oidc refresh failed: {e}");
                }
            }
        }

        // Client credentials can acquire directly
        match &self.flow {
            OidcFlow::ClientCredentials {
                client_secret,
                token_auth,
            } => {
                let discovery = self.get_discovery().await?;
                self.do_client_credentials_acquire(discovery, client_secret, *token_auth)
                    .await?;
                // Re-read from cache
                let (at, _, exp) = self.read_entry(&key).await;
                at.map(|s| cli_framework::auth::AccessToken::new(s, exp))
                    .ok_or(cli_framework::auth::AuthError::NotAuthenticated)
            }
            _ => Err(cli_framework::auth::AuthError::NotAuthenticated),
        }
    }

    async fn do_refresh(
        &self,
        discovery: &DiscoveryDoc,
        refresh_token: &str,
    ) -> Result<cli_framework::auth::AccessToken, cli_framework::auth::AuthError> {
        let params = [
            ("grant_type", "refresh_token"),
            ("client_id", self.client_id.as_str()),
            ("refresh_token", refresh_token),
        ];
        let resp: serde_json::Value = self
            .http
            .post(&discovery.token_endpoint)
            .form(&params)
            .send()
            .await
            .map_err(|e| cli_framework::auth::AuthError::Provider {
                message: e.to_string(),
                source: Some(Box::new(e)),
            })?
            .json()
            .await
            .map_err(|e| cli_framework::auth::AuthError::Provider {
                message: e.to_string(),
                source: Some(Box::new(e)),
            })?;

        self.store_token_response(&resp).await?;

        let key = self.cache_key();
        let (at, _, exp) = self.read_entry(&key).await;

        at.map(|s| cli_framework::auth::AccessToken::new(s, exp))
            .ok_or(cli_framework::auth::AuthError::NotAuthenticated)
    }

    async fn do_interactive_login(&self) -> Result<(), cli_framework::auth::AuthError> {
        let discovery = self.get_discovery().await?;
        match &self.flow {
            OidcFlow::DeviceCode => self.do_device_code_login(discovery).await,
            OidcFlow::AuthCodePkce { redirect } => {
                let redirect = redirect.clone();
                self.do_auth_code_pkce_login(discovery, &redirect).await
            }
            OidcFlow::ClientCredentials {
                client_secret,
                token_auth,
            } => {
                self.do_client_credentials_acquire(discovery, client_secret, *token_auth)
                    .await
            }
        }
    }

    async fn do_device_code_login(
        &self,
        discovery: &DiscoveryDoc,
    ) -> Result<(), cli_framework::auth::AuthError> {
        let endpoint =
            discovery
                .device_authorization_endpoint
                .as_deref()
                .ok_or_else(|| cli_framework::auth::AuthError::Provider {
                    message: "provider does not advertise device_authorization_endpoint required for device_code".to_string(),
                    source: None,
                })?;

        let scopes = self.effective_scopes();
        let scope_str = scopes.join(" ");

        // PKCE on the device flow (RFC 8628 + RFC 7636), sent UNCONDITIONALLY.
        //
        // A provider that mandates PKCE rejects the device-authorization request
        // outright when it is absent — Keycloak with the client attribute
        // `pkce.code.challenge.method: S256` answers
        // `invalid_request: Missing parameter: code_challenge_method`, so the
        // whole flow is dead before a user code is ever shown. Sending it to a
        // provider that does NOT mandate it is harmless: the challenge is stored
        // and the verifier checked at redemption, which is strictly better than
        // not binding the device code at all. Both halves verified end to end
        // against Keycloak 26.6.3 with two clients, one with the attribute and
        // one without; a mismatched verifier is rejected with
        // `invalid_grant: PKCE verification failed`.
        let code_verifier = crate::pkce::generate_verifier();
        let code_challenge = crate::pkce::derive_challenge(&code_verifier);

        let params = [
            ("client_id", self.client_id.as_str()),
            ("scope", scope_str.as_str()),
            ("code_challenge", code_challenge.as_str()),
            ("code_challenge_method", "S256"),
        ];

        let resp: serde_json::Value = self
            .http
            .post(endpoint)
            .form(&params)
            .send()
            .await
            .map_err(|e| cli_framework::auth::AuthError::Provider {
                message: e.to_string(),
                source: Some(Box::new(e)),
            })?
            .json()
            .await
            .map_err(|e| cli_framework::auth::AuthError::Provider {
                message: e.to_string(),
                source: Some(Box::new(e)),
            })?;

        // The device-authorization response is an error object or a grant; it was
        // previously read as neither. `unwrap_or("")` on every field turned a
        // rejection into an empty user code printed to the operator followed by a
        // poll loop on an empty device code, so the surfaced failure was a
        // downstream `invalid_grant` that named nothing about the real cause.
        if let Some(error) = resp["error"].as_str() {
            let desc = resp["error_description"].as_str().unwrap_or("");
            return Err(cli_framework::auth::AuthError::Provider {
                message: if desc.is_empty() {
                    format!("device authorization request rejected: {error}")
                } else {
                    format!("device authorization request rejected: {error}: {desc}")
                },
                source: None,
            });
        }

        let missing = |field: &str| cli_framework::auth::AuthError::Provider {
            message: format!("device authorization response missing `{field}`"),
            source: None,
        };
        let verification_uri = resp["verification_uri"]
            .as_str()
            .ok_or_else(|| missing("verification_uri"))?;
        let user_code = resp["user_code"]
            .as_str()
            .ok_or_else(|| missing("user_code"))?;
        let device_code = resp["device_code"]
            .as_str()
            .ok_or_else(|| missing("device_code"))?
            .to_string();
        let interval_secs = resp["interval"].as_u64().unwrap_or(5);
        let expires_in = resp["expires_in"].as_u64().unwrap_or(600);

        self.reporter.user_code(verification_uri, user_code);

        let deadline = tokio::time::Instant::now() + Duration::from_secs(expires_in);
        let mut poll_interval = Duration::from_secs(interval_secs);

        loop {
            if tokio::time::Instant::now() > deadline {
                return Err(cli_framework::auth::AuthError::Provider {
                    message: "device code expired".to_string(),
                    source: None,
                });
            }
            tokio::time::sleep(poll_interval).await;

            let poll_params = [
                ("client_id", self.client_id.as_str()),
                ("device_code", device_code.as_str()),
                ("grant_type", "urn:ietf:params:oauth:grant-type:device_code"),
                ("code_verifier", code_verifier.as_str()),
            ];
            let resp: serde_json::Value = self
                .http
                .post(&discovery.token_endpoint)
                .form(&poll_params)
                .send()
                .await
                .map_err(|e| cli_framework::auth::AuthError::Provider {
                    message: e.to_string(),
                    source: Some(Box::new(e)),
                })?
                .json()
                .await
                .map_err(|e| cli_framework::auth::AuthError::Provider {
                    message: e.to_string(),
                    source: Some(Box::new(e)),
                })?;

            if let Some(error) = resp["error"].as_str() {
                match error {
                    "authorization_pending" => continue,
                    "slow_down" => {
                        poll_interval += Duration::from_secs(5);
                        continue;
                    }
                    _ => {
                        let desc = resp["error_description"].as_str().unwrap_or("");
                        return Err(cli_framework::auth::AuthError::Provider {
                            message: format!("{error}: {desc}"),
                            source: None,
                        });
                    }
                }
            }

            self.store_token_response(&resp).await?;
            return Ok(());
        }
    }

    async fn do_auth_code_pkce_login(
        &self,
        discovery: &DiscoveryDoc,
        redirect: &RedirectConfig,
    ) -> Result<(), cli_framework::auth::AuthError> {
        // PKCE + CSRF state, both from the OS CSPRNG (see `crate::pkce`).
        let code_verifier = crate::pkce::generate_verifier();
        let code_challenge = crate::pkce::derive_challenge(&code_verifier);
        let state = crate::pkce::generate_state();

        let port = match redirect.port {
            RedirectPort::Fixed(p) => p,
            RedirectPort::Ephemeral => 0,
        };

        let listener = std::net::TcpListener::bind(("127.0.0.1", port)).map_err(|e| {
            cli_framework::auth::AuthError::Provider {
                message: format!("bind loopback: {e}"),
                source: Some(Box::new(e)),
            }
        })?;
        let actual_port = listener.local_addr().unwrap().port();
        let redirect_uri = format!("http://127.0.0.1:{}/callback", actual_port);

        let auth_endpoint = discovery.authorization_endpoint.as_deref().ok_or_else(|| {
            cli_framework::auth::AuthError::Provider {
                message:
                    "provider does not advertise authorization_endpoint required for auth_code_pkce"
                        .to_string(),
                source: None,
            }
        })?;

        let scopes = self.effective_scopes().join(" ");
        let auth_url = format!(
            "{}?response_type=code&client_id={}&redirect_uri={}&scope={}&state={}&code_challenge={}&code_challenge_method=S256",
            auth_endpoint,
            percent_encode(&self.client_id),
            percent_encode(&redirect_uri),
            percent_encode(&scopes),
            &state,
            &code_challenge,
        );

        if open::that(&auth_url).is_err() {
            self.reporter
                .message(&format!("Open this URL to log in: {}", auth_url));
        }

        let expected_state = state.clone();
        let code = wait_for_callback(listener, &expected_state, Duration::from_secs(300)).map_err(
            |e| cli_framework::auth::AuthError::Provider {
                message: e,
                source: None,
            },
        )?;

        let params = [
            ("grant_type", "authorization_code"),
            ("client_id", self.client_id.as_str()),
            ("code", code.as_str()),
            ("redirect_uri", redirect_uri.as_str()),
            ("code_verifier", code_verifier.as_str()),
        ];
        let resp: serde_json::Value = self
            .http
            .post(&discovery.token_endpoint)
            .form(&params)
            .send()
            .await
            .map_err(|e| cli_framework::auth::AuthError::Provider {
                message: e.to_string(),
                source: Some(Box::new(e)),
            })?
            .json()
            .await
            .map_err(|e| cli_framework::auth::AuthError::Provider {
                message: e.to_string(),
                source: Some(Box::new(e)),
            })?;

        self.store_token_response(&resp).await
    }

    async fn do_client_credentials_acquire(
        &self,
        discovery: &DiscoveryDoc,
        client_secret: &SecretString,
        token_auth: TokenAuthMethod,
    ) -> Result<(), cli_framework::auth::AuthError> {
        use secrecy::ExposeSecret;

        let scopes = self.effective_scopes();
        let scope_str = scopes.join(" ");

        let resp: serde_json::Value = match token_auth {
            TokenAuthMethod::Post => {
                let mut params = vec![
                    ("grant_type", "client_credentials"),
                    ("client_id", self.client_id.as_str()),
                    ("client_secret", client_secret.expose_secret()),
                ];
                if !scope_str.is_empty() {
                    params.push(("scope", scope_str.as_str()));
                }
                self.http
                    .post(&discovery.token_endpoint)
                    .form(&params)
                    .send()
                    .await
                    .map_err(|e| cli_framework::auth::AuthError::Provider {
                        message: e.to_string(),
                        source: Some(Box::new(e)),
                    })?
                    .json()
                    .await
                    .map_err(|e| cli_framework::auth::AuthError::Provider {
                        message: e.to_string(),
                        source: Some(Box::new(e)),
                    })?
            }
            TokenAuthMethod::Basic => {
                let mut params = vec![("grant_type", "client_credentials")];
                if !scope_str.is_empty() {
                    params.push(("scope", scope_str.as_str()));
                }
                self.http
                    .post(&discovery.token_endpoint)
                    .basic_auth(&self.client_id, Some(client_secret.expose_secret()))
                    .form(&params)
                    .send()
                    .await
                    .map_err(|e| cli_framework::auth::AuthError::Provider {
                        message: e.to_string(),
                        source: Some(Box::new(e)),
                    })?
                    .json()
                    .await
                    .map_err(|e| cli_framework::auth::AuthError::Provider {
                        message: e.to_string(),
                        source: Some(Box::new(e)),
                    })?
            }
        };

        self.store_token_response(&resp).await
    }

    async fn store_token_response(
        &self,
        resp: &serde_json::Value,
    ) -> Result<(), cli_framework::auth::AuthError> {
        if let Some(error) = resp["error"].as_str() {
            let desc = resp["error_description"].as_str().unwrap_or("");
            return Err(cli_framework::auth::AuthError::Provider {
                message: format!("{error}: {desc}"),
                source: None,
            });
        }

        let access_token = resp["access_token"].as_str().ok_or_else(|| {
            cli_framework::auth::AuthError::Provider {
                message: "missing access_token".to_string(),
                source: None,
            }
        })?;

        let token_type = resp["token_type"].as_str().unwrap_or("");
        if !token_type.eq_ignore_ascii_case("bearer") {
            return Err(cli_framework::auth::AuthError::Provider {
                message: format!("expected token_type=Bearer, got {token_type}"),
                source: None,
            });
        }

        let expires_at = resp["expires_in"]
            .as_u64()
            .map(|secs| SystemTime::now() + Duration::from_secs(secs));

        let refresh_token = resp["refresh_token"].as_str().map(String::from);

        let key = self.cache_key();
        let access_token = access_token.to_string();
        let scopes = self.effective_scopes();

        let lock = lock_cache_dir(&self.cache_dir).await?;

        let mut cache = read_cache(self.secret_store.as_ref(), &self.cache_secret_key).await;
        let existing_refresh = cache
            .entries
            .get(&key)
            .and_then(|e| e.refresh_token.clone());

        let entry = CacheEntry {
            access_token: Some(access_token),
            refresh_token: refresh_token.or(existing_refresh),
            expires_at: expires_at.map(cache::format_rfc3339),
            obtained_at: cache::format_rfc3339(SystemTime::now()),
            scopes,
        };
        cache.entries.insert(key, entry);

        if let Err(e) =
            write_cache(self.secret_store.as_ref(), &self.cache_secret_key, &cache).await
        {
            tracing::warn!("oidc token cache: write failed: {e}");
        }
        unlock_cache_dir(lock);
        Ok(())
    }
}

// ── TokenProvider impl ──────────────────────────────────────────────────────

#[async_trait]
impl cli_framework::auth::TokenProvider for OidcClient {
    async fn token(
        &self,
    ) -> Result<cli_framework::auth::AccessToken, cli_framework::auth::AuthError> {
        self.get_or_refresh_token().await
    }

    async fn invalidate(&self) {
        let key = self.cache_key();
        let lock = match lock_cache_dir(&self.cache_dir).await {
            Ok(l) => l,
            Err(e) => {
                tracing::warn!("oidc token cache: invalidate lock failed: {e}");
                return;
            }
        };
        let mut cache = read_cache(self.secret_store.as_ref(), &self.cache_secret_key).await;
        if let Some(entry) = cache.entries.get_mut(&key) {
            entry.access_token = None;
            entry.expires_at = None;
        }
        if let Err(e) =
            write_cache(self.secret_store.as_ref(), &self.cache_secret_key, &cache).await
        {
            tracing::warn!("oidc token cache: invalidate write failed: {e}");
        }
        unlock_cache_dir(lock);
    }

    async fn peek(&self) -> Option<cli_framework::auth::TokenStatus> {
        let key = self.cache_key();
        let cache = read_cache(self.secret_store.as_ref(), &self.cache_secret_key).await;
        let entry = cache.entries.get(&key).cloned()?;
        let has_access = entry.access_token.is_some();
        let has_refresh = entry.refresh_token.is_some();
        let expires_at = entry.expires_at.as_deref().and_then(cache::parse_rfc3339);
        Some(cli_framework::auth::TokenStatus {
            logged_in: has_access || has_refresh,
            expires_at: if has_access { expires_at } else { None },
        })
    }

    async fn login(&self) -> Result<(), cli_framework::auth::AuthError> {
        self.do_interactive_login().await
    }

    async fn logout(&self) -> Result<(), cli_framework::auth::AuthError> {
        let key = self.cache_key();
        let lock = lock_cache_dir(&self.cache_dir).await?;
        let mut cache = read_cache(self.secret_store.as_ref(), &self.cache_secret_key).await;
        cache.entries.remove(&key);
        if let Err(e) =
            write_cache(self.secret_store.as_ref(), &self.cache_secret_key, &cache).await
        {
            tracing::warn!("oidc token cache: logout write failed: {e}");
        }
        unlock_cache_dir(lock);
        Ok(())
    }
}

// ── OidcClientBuilder ───────────────────────────────────────────────────────

pub struct OidcClientBuilder {
    issuer_url: Option<String>,
    client_id: Option<String>,
    flow: Option<OidcFlow>,
    scopes: Option<Vec<String>>,
    cache_dir: Option<PathBuf>,
    app_name: Option<String>,
    reporter: Option<Arc<dyn cli_framework::auth::AuthFlowReporter>>,
    refresh_skew: Duration,
    secret_store: Option<Arc<dyn SecretStore>>,
}

impl OidcClientBuilder {
    fn new() -> Self {
        Self {
            issuer_url: None,
            client_id: None,
            flow: None,
            scopes: None,
            cache_dir: None,
            app_name: None,
            reporter: None,
            refresh_skew: Duration::from_secs(60),
            secret_store: None,
        }
    }

    /// Build from environment variables with a `{PREFIX}_` namespace:
    ///
    /// | Var | Required | Meaning |
    /// |-----|----------|---------|
    /// | `{PREFIX}_ISSUER_URL` | yes | OIDC issuer / Keycloak realm URL |
    /// | `{PREFIX}_CLIENT_ID` | yes | OAuth client id |
    /// | `{PREFIX}_CLIENT_SECRET` | no | Confidential-client secret (implies Client Credentials) |
    /// | `{PREFIX}_FLOW` | no | `device` \| `pkce` \| `client-credentials` \| `auto` |
    /// | `{PREFIX}_SCOPES` | no | Space- or comma-separated scopes |
    ///
    /// Flow resolution: an explicit `{PREFIX}_FLOW` wins; otherwise a present
    /// secret selects Client Credentials, and its absence selects an interactive
    /// flow via [`OidcFlow::auto_interactive`]. Returns the builder so callers can
    /// still override (e.g. `.app_name(..)`) before [`build`](Self::build).
    pub fn from_env(prefix: &str) -> Result<Self, OidcConfigError> {
        let var = |k: &str| {
            std::env::var(format!("{prefix}_{k}"))
                .ok()
                .filter(|v| !v.is_empty())
        };

        let issuer_url =
            var("ISSUER_URL").ok_or(OidcConfigError::MissingField("ISSUER_URL (env)"))?;
        let client_id = var("CLIENT_ID").ok_or(OidcConfigError::MissingField("CLIENT_ID (env)"))?;
        let secret = var("CLIENT_SECRET");
        let flow_kind = var("FLOW");

        let flow = match flow_kind.as_deref() {
            Some("device") => OidcFlow::DeviceCode,
            Some("pkce") => OidcFlow::AuthCodePkce {
                redirect: RedirectConfig::default(),
            },
            Some("client-credentials") | Some("cc") => OidcFlow::ClientCredentials {
                client_secret: SecretString::new(secret.clone().ok_or_else(|| {
                    OidcConfigError::InvalidFlow(
                        "client-credentials flow requires CLIENT_SECRET".to_string(),
                    )
                })?),
                token_auth: TokenAuthMethod::Post,
            },
            Some("auto") | None => match &secret {
                Some(s) => OidcFlow::ClientCredentials {
                    client_secret: SecretString::new(s.clone()),
                    token_auth: TokenAuthMethod::Post,
                },
                None => OidcFlow::auto_interactive(),
            },
            Some(other) => {
                return Err(OidcConfigError::InvalidFlow(format!(
                    "unknown {prefix}_FLOW value: {other}"
                )))
            }
        };

        let mut builder = Self::new()
            .issuer_url(issuer_url)
            .client_id(client_id)
            .flow(flow);
        if let Some(raw) = var("SCOPES") {
            let scopes: Vec<String> = raw
                .split([',', ' '])
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_string)
                .collect();
            if !scopes.is_empty() {
                builder = builder.scopes(scopes);
            }
        }
        Ok(builder)
    }

    pub fn issuer_url(mut self, url: impl Into<String>) -> Self {
        self.issuer_url = Some(url.into());
        self
    }

    pub fn client_id(mut self, id: impl Into<String>) -> Self {
        self.client_id = Some(id.into());
        self
    }

    pub fn flow(mut self, flow: OidcFlow) -> Self {
        self.flow = Some(flow);
        self
    }

    pub fn scopes(mut self, scopes: Vec<String>) -> Self {
        self.scopes = Some(scopes);
        self
    }

    pub fn cache_dir(mut self, dir: PathBuf) -> Self {
        self.cache_dir = Some(dir);
        self
    }

    /// Application name used to derive a default cache directory when
    /// [`cache_dir`](Self::cache_dir) is not set: `<os-cache>/cli-framework-oidc/<app-name>`.
    pub fn app_name(mut self, name: impl Into<String>) -> Self {
        self.app_name = Some(name.into());
        self
    }

    pub fn reporter(mut self, r: Arc<dyn cli_framework::auth::AuthFlowReporter>) -> Self {
        self.reporter = Some(r);
        self
    }

    pub fn refresh_skew(mut self, d: Duration) -> Self {
        self.refresh_skew = d;
        self
    }

    /// Where the token cache is stored. Defaults to an
    /// [`EnvFileSecretStore`] rooted at `cache_dir`, which reproduces the
    /// crate's historical zero-config on-disk `oidc-token.json` behavior
    /// exactly. Inject e.g. `secrets-openbao::OpenBaoSecretStore` here to
    /// store cached tokens in a real secrets manager instead — when a
    /// non-file backend is configured, no plaintext token file is ever
    /// written.
    pub fn secret_store(mut self, store: Arc<dyn SecretStore>) -> Self {
        self.secret_store = Some(store);
        self
    }

    pub fn build(self) -> Result<OidcClient, OidcConfigError> {
        let issuer_url = crate::normalize_issuer(
            self.issuer_url
                .as_deref()
                .ok_or(OidcConfigError::MissingField("issuer_url"))?,
        )?;
        let client_id = self
            .client_id
            .ok_or(OidcConfigError::MissingField("client_id"))?;
        let flow = self.flow.ok_or(OidcConfigError::MissingField("flow"))?;
        let cache_dir = match self.cache_dir {
            Some(dir) => dir,
            None => default_cache_dir(self.app_name.as_deref())?,
        };
        let reporter = self
            .reporter
            .unwrap_or_else(|| Arc::new(cli_framework::auth::StderrAuthFlowReporter));
        let secret_store = self
            .secret_store
            .unwrap_or_else(|| Arc::new(EnvFileSecretStore::new(cache_dir.clone())));

        Ok(OidcClient {
            issuer_url,
            client_id,
            flow,
            scopes: self.scopes,
            cache_dir,
            reporter,
            refresh_skew: self.refresh_skew,
            discovery: tokio::sync::OnceCell::new(),
            http: make_http_client(),
            secret_store,
            cache_secret_key: cache_secret_key(),
        })
    }
}

// ── Helpers ─────────────────────────────────────────────────────────────────

/// Resolve the default token-cache directory: `<os-cache>/cli-framework-oidc/<app-name>`.
/// `app_name` falls back to `"default"` when not supplied.
fn default_cache_dir(app_name: Option<&str>) -> Result<PathBuf, OidcConfigError> {
    let base = dirs::cache_dir().ok_or(OidcConfigError::MissingField("cache_dir"))?;
    Ok(base
        .join("cli-framework-oidc")
        .join(app_name.unwrap_or("default")))
}

/// Best-effort cross-process advisory lock guarding the read-modify-write
/// cycle around the token cache (`<cache_dir>/oidc-token.lock`).
///
/// This is independent of which `SecretStore` backend is configured: it
/// still serializes concurrent writers *on this host* even when the backend
/// is remote (e.g. OpenBao) — not a substitute for backend-side optimistic
/// concurrency, which is out of scope for R1. Acquiring it never blocks the
/// caller's task directly (runs via `spawn_blocking`); releasing it
/// (`unlock_cache_dir`) is a fast local syscall done inline.
async fn lock_cache_dir(
    cache_dir: &std::path::Path,
) -> Result<std::fs::File, cli_framework::auth::AuthError> {
    let cache_dir = cache_dir.to_path_buf();
    tokio::task::spawn_blocking(move || {
        std::fs::create_dir_all(&cache_dir).ok();
        let lock_path = cache_dir.join("oidc-token.lock");
        let file = open_lock_file(&lock_path)?;
        use fs2::FileExt;
        file.lock_exclusive()?;
        Ok::<_, std::io::Error>(file)
    })
    .await
    .map_err(|e| cli_framework::auth::AuthError::Provider {
        message: e.to_string(),
        source: None,
    })?
    .map_err(|e| cli_framework::auth::AuthError::Provider {
        message: format!("oidc token cache: lock failed: {e}"),
        source: Some(Box::new(e)),
    })
}

fn unlock_cache_dir(file: std::fs::File) {
    #[allow(clippy::incompatible_msrv)]
    let _ = file.unlock();
}

fn open_lock_file(path: &std::path::Path) -> std::io::Result<std::fs::File> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        std::fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .mode(0o600)
            .open(path)
    }
    #[cfg(not(unix))]
    {
        std::fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(path)
    }
}

fn make_http_client() -> reqwest::Client {
    reqwest::Client::builder()
        .user_agent(concat!("cli-framework-oidc/", env!("CARGO_PKG_VERSION")))
        .build()
        .expect("failed to build reqwest client")
}

async fn fetch_discovery(
    issuer_url: &str,
    http: &reqwest::Client,
) -> Result<DiscoveryDoc, cli_framework::auth::AuthError> {
    let url = format!("{}/.well-known/openid-configuration", issuer_url);
    let resp =
        http.get(&url)
            .send()
            .await
            .map_err(|e| cli_framework::auth::AuthError::Provider {
                message: e.to_string(),
                source: Some(Box::new(e)),
            })?;
    let doc: serde_json::Value =
        resp.json()
            .await
            .map_err(|e| cli_framework::auth::AuthError::Provider {
                message: e.to_string(),
                source: Some(Box::new(e)),
            })?;

    let doc_issuer = doc["issuer"].as_str().unwrap_or("");
    if crate::normalize_issuer(doc_issuer).ok().as_deref() != Some(issuer_url) {
        return Err(cli_framework::auth::AuthError::Provider {
            message: format!("discovery document issuer mismatch: got {doc_issuer}"),
            source: None,
        });
    }

    Ok(DiscoveryDoc {
        token_endpoint: doc["token_endpoint"].as_str().unwrap_or("").to_string(),
        device_authorization_endpoint: doc["device_authorization_endpoint"]
            .as_str()
            .map(String::from),
        authorization_endpoint: doc["authorization_endpoint"].as_str().map(String::from),
    })
}

fn percent_encode(s: &str) -> String {
    url::form_urlencoded::byte_serialize(s.as_bytes()).collect()
}

fn wait_for_callback(
    listener: std::net::TcpListener,
    expected_state: &str,
    timeout: Duration,
) -> Result<String, String> {
    use std::io::{BufRead, BufReader, Write};

    // Accept with a deadline: set nonblocking + poll until timeout.
    listener.set_nonblocking(true).map_err(|e| e.to_string())?;
    let deadline = std::time::Instant::now() + timeout;
    let (mut stream, _) = loop {
        match listener.accept() {
            Ok(pair) => break pair,
            Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                if std::time::Instant::now() >= deadline {
                    return Err("callback timeout".to_string());
                }
                std::thread::sleep(Duration::from_millis(50));
            }
            Err(e) => return Err(e.to_string()),
        }
    };
    listener.set_nonblocking(false).ok();
    stream
        .set_read_timeout(Some(Duration::from_secs(10)))
        .map_err(|e| e.to_string())?;

    let reader = BufReader::new(stream.try_clone().map_err(|e| e.to_string())?);
    let first_line = reader
        .lines()
        .next()
        .ok_or("no request")?
        .map_err(|e| e.to_string())?;

    // GET /callback?code=...&state=...
    let path = first_line.split_whitespace().nth(1).unwrap_or("");
    let query = path.split('?').nth(1).unwrap_or("");

    let mut code = None;
    let mut state_got = None;
    for pair in query.split('&') {
        let mut kv = pair.splitn(2, '=');
        match (kv.next(), kv.next()) {
            (Some("code"), Some(v)) => {
                code = Some(decode_param(v));
            }
            (Some("state"), Some(v)) => {
                state_got = Some(decode_param(v));
            }
            _ => {}
        }
    }

    let response = "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\n\r\n<html><body>Login complete. You may close this window.</body></html>";
    let _ = stream.write_all(response.as_bytes());

    if state_got.as_deref() != Some(expected_state) {
        return Err("state mismatch".to_string());
    }
    code.ok_or_else(|| "no code in callback".to_string())
}

fn decode_param(s: &str) -> String {
    url::form_urlencoded::parse(format!("x={}", s).as_bytes())
        .next()
        .map(|(_, v)| v.to_string())
        .unwrap_or_else(|| s.to_string())
}

#[cfg(test)]
mod flow_selection_tests {
    use super::*;

    #[test]
    fn remote_session_picks_device_code() {
        // Over SSH the browser would open on the wrong machine → Device Code.
        assert!(matches!(
            pick_interactive_flow(true, true),
            OidcFlow::DeviceCode
        ));
        assert!(matches!(
            pick_interactive_flow(true, false),
            OidcFlow::DeviceCode
        ));
    }

    #[test]
    fn local_gui_picks_pkce() {
        assert!(matches!(
            pick_interactive_flow(false, true),
            OidcFlow::AuthCodePkce { .. }
        ));
    }

    #[test]
    fn local_headless_picks_device_code() {
        assert!(matches!(
            pick_interactive_flow(false, false),
            OidcFlow::DeviceCode
        ));
    }
}
