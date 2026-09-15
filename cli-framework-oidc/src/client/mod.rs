//! OIDC client: `OidcClient` implementing `cli_framework::auth::TokenProvider`.

use crate::OidcConfigError;
use async_trait::async_trait;
use cli_framework::secrets::{EnvFileSecretStore, SecretKey, SecretStore};
use secrecy::SecretString;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, SystemTime};

mod cache;
mod callback;
pub use cache::{default_cache_secret_key, legacy_cache_secret_key};
use cache::{read_cache, write_cache, CacheEntry};

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
    open_browser: bool,
    refresh_skew: Duration,
    discovery: tokio::sync::OnceCell<DiscoveryDoc>,
    http: reqwest::Client,
    /// Where the token cache is stored. Defaults to an
    /// [`EnvFileSecretStore`] rooted at `cache_dir`. Inject a different
    /// backend (e.g. OpenBao-backed) via [`OidcClientBuilder::secret_store`]
    /// to store cached tokens somewhere other than a local file.
    secret_store: Arc<dyn SecretStore>,
    cache_secret_key: SecretKey,
}

impl OidcClient {
    /// The on-disk directory holding this client's token cache.
    /// Either the explicit `cache_dir` or the app-name-derived default.
    pub fn cache_dir(&self) -> &std::path::Path {
        &self.cache_dir
    }

    /// `SecretStore` key for the token cache (`<app>/oidc/token.json` by default).
    pub fn cache_secret_key(&self) -> &SecretKey {
        &self.cache_secret_key
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

        // Client credentials can acquire a new grant without a user.
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
            _ => {
                // Interactive flows cannot re-prompt here. A just-issued
                // access token is often already inside `refresh_skew` of
                // expiry (Keycloak access TTL of 60s vs the 60s default
                // skew). Returning NotAuthenticated after a successful
                // login() is wrong: the user just authenticated.
                if let Some(at) = access_token {
                    let not_expired = expires_at.is_none_or(|exp| SystemTime::now() < exp);
                    if not_expired {
                        return Ok(cli_framework::auth::AccessToken::new(at, expires_at));
                    }
                }
                Err(cli_framework::auth::AuthError::NotAuthenticated)
            }
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

        let listener = tokio::net::TcpListener::bind(("127.0.0.1", port))
            .await
            .map_err(|e| cli_framework::auth::AuthError::Provider {
                message: format!("bind loopback: {e}"),
                source: Some(Box::new(e)),
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
        let auth_url = callback::authorization_url(
            auth_endpoint,
            &self.client_id,
            &redirect_uri,
            &scopes,
            &state,
            &code_challenge,
        )
        .map_err(|message| cli_framework::auth::AuthError::Provider {
            message,
            source: None,
        })?;

        self.reporter
            .message(&format!("Open this URL to log in: {}", auth_url));
        if self.open_browser {
            // A detached OS thread neither delays callback acceptance nor keeps
            // Tokio runtime shutdown waiting on a stuck platform launcher.
            let _ = std::thread::Builder::new()
                .name("oidc-browser".into())
                .spawn(move || {
                    let _ = open::that(auth_url);
                });
        }

        let expected_state = state.clone();
        let code = callback::wait_for_callback(listener, &expected_state, Duration::from_secs(300))
            .await
            .map_err(|e| cli_framework::auth::AuthError::Provider {
                message: e,
                source: None,
            })?;

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

        let lock = lock_cache_dir(&self.cache_dir, &self.cache_secret_key).await?;

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

        let write = write_cache(self.secret_store.as_ref(), &self.cache_secret_key, &cache).await;
        unlock_cache_dir(lock);
        write.map_err(|e| cli_framework::auth::AuthError::Provider {
            message: format!("token cache write failed: {e}"),
            source: None,
        })?;
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
        let lock = match lock_cache_dir(&self.cache_dir, &self.cache_secret_key).await {
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
        let lock = lock_cache_dir(&self.cache_dir, &self.cache_secret_key).await?;
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
    open_browser: bool,
    refresh_skew: Duration,
    secret_store: Option<Arc<dyn SecretStore>>,
    cache_secret_key: Option<SecretKey>,
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
            open_browser: true,
            refresh_skew: Duration::from_secs(60),
            secret_store: None,
            cache_secret_key: None,
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

    /// Choose automatic browser launch for PKCE login (default: true).
    /// The authorization URL is always reported for manual opening. Setting
    /// false guarantees this client does not invoke a platform browser launcher.
    pub fn open_browser(mut self, enabled: bool) -> Self {
        self.open_browser = enabled;
        self
    }

    pub fn refresh_skew(mut self, d: Duration) -> Self {
        self.refresh_skew = d;
        self
    }

    /// Where the token cache is stored. Defaults to an
    /// [`EnvFileSecretStore`] rooted at `cache_dir`. Inject e.g.
    /// `secrets-openbao::OpenBaoSecretStore` here to store cached tokens in
    /// a real secrets manager instead — when a non-file backend is
    /// configured, no plaintext token file is ever written.
    pub fn secret_store(mut self, store: Arc<dyn SecretStore>) -> Self {
        self.secret_store = Some(store);
        self
    }

    /// Override the `SecretStore` key used for the token cache.
    ///
    /// Default is [`default_cache_secret_key`] from [`Self::app_name`]
    /// (`<app>/oidc/token.json`). Use this when one process hosts more than
    /// one OIDC client against the same store (for example management vs
    /// product sign-in).
    pub fn cache_secret_key(mut self, key: SecretKey) -> Self {
        self.cache_secret_key = Some(key);
        self
    }

    pub fn build(self) -> Result<OidcClient, OidcConfigError> {
        let raw_issuer = self
            .issuer_url
            .as_deref()
            .ok_or(OidcConfigError::MissingField("issuer_url"))?;
        crate::endpoint_security::secure_issuer(raw_issuer)
            .map_err(|()| OidcConfigError::InsecureIssuer("unsafe issuer URL".to_owned()))?;
        let issuer_url = crate::normalize_issuer(raw_issuer)?;
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
        let cache_secret_key = self
            .cache_secret_key
            .unwrap_or_else(|| default_cache_secret_key(self.app_name.as_deref()));

        Ok(OidcClient {
            issuer_url,
            client_id,
            flow,
            scopes: self.scopes,
            cache_dir,
            reporter,
            open_browser: self.open_browser,
            refresh_skew: self.refresh_skew,
            discovery: tokio::sync::OnceCell::new(),
            http: make_http_client()?,
            secret_store,
            cache_secret_key,
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
/// cycle around the token cache (`<cache_dir>/<app>/oidc/token.lock`).
///
/// This is independent of which `SecretStore` backend is configured: it
/// still serializes concurrent writers *on this host* even when the backend
/// is remote (e.g. OpenBao) — not a substitute for backend-side optimistic
/// concurrency, which is out of scope for R1. Acquiring it never blocks the
/// caller's task directly (runs via `spawn_blocking`); releasing it
/// (`unlock_cache_dir`) is a fast local syscall done inline.
async fn lock_cache_dir(
    cache_dir: &std::path::Path,
    key: &SecretKey,
) -> Result<std::fs::File, cli_framework::auth::AuthError> {
    let cache_dir = cache_dir.to_path_buf();
    let lock_rel = cache::cache_lock_relpath(key);
    tokio::task::spawn_blocking(move || {
        let lock_path = cache_dir.join(lock_rel);
        if let Some(parent) = lock_path.parent() {
            std::fs::create_dir_all(parent).ok();
        }
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

/// Build the sole native-client transport.
///
/// No redirect is followed: in particular, a 307/308 response must never
/// replay an authorization code, verifier, refresh token, device code, or
/// client secret to a different authority. The total request deadline applies
/// to discovery and every grant request.
fn make_http_client() -> Result<reqwest::Client, OidcConfigError> {
    reqwest::Client::builder()
        .user_agent(concat!("cli-framework-oidc/", env!("CARGO_PKG_VERSION")))
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(15))
        .build()
        .map_err(|_| {
            OidcConfigError::InvalidFlow(
                "could not initialize bounded OIDC HTTP transport".to_owned(),
            )
        })
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
    if !resp.status().is_success() {
        return Err(discovery_failure(
            "OIDC discovery returned a non-success status",
        ));
    }
    let doc: serde_json::Value = resp
        .json()
        .await
        .map_err(|_| discovery_failure("OIDC discovery returned invalid JSON"))?;

    let doc_issuer = doc["issuer"].as_str().unwrap_or("");
    if crate::endpoint_security::secure_issuer(doc_issuer).is_err()
        || crate::normalize_issuer(doc_issuer).ok().as_deref() != Some(issuer_url)
    {
        return Err(discovery_failure("OIDC discovery issuer mismatch"));
    }

    let token_endpoint = required_discovered_endpoint(&doc, "token_endpoint")?;
    let device_authorization_endpoint =
        optional_discovered_endpoint(&doc, "device_authorization_endpoint")?;
    let authorization_endpoint = optional_discovered_endpoint(&doc, "authorization_endpoint")?;
    Ok(DiscoveryDoc {
        token_endpoint,
        device_authorization_endpoint,
        authorization_endpoint,
    })
}

/// Read and validate a required metadata endpoint before any credential exists.
fn required_discovered_endpoint(
    document: &serde_json::Value,
    field: &'static str,
) -> Result<String, cli_framework::auth::AuthError> {
    let raw = document[field]
        .as_str()
        .filter(|value| !value.is_empty())
        .ok_or_else(|| discovery_failure("OIDC discovery omitted a required endpoint"))?;
    crate::endpoint_security::secure_endpoint(raw)
        .map_err(|()| discovery_failure("OIDC discovery advertised an unsafe endpoint"))?;
    Ok(raw.to_owned())
}

/// Validate an optional metadata endpoint when the provider advertises it.
fn optional_discovered_endpoint(
    document: &serde_json::Value,
    field: &'static str,
) -> Result<Option<String>, cli_framework::auth::AuthError> {
    match &document[field] {
        serde_json::Value::Null => Ok(None),
        serde_json::Value::String(_) => required_discovered_endpoint(document, field).map(Some),
        _ => Err(discovery_failure(
            "OIDC discovery advertised a malformed endpoint",
        )),
    }
}

/// Keep metadata failures stable without reflecting attacker-controlled URLs.
fn discovery_failure(message: &'static str) -> cli_framework::auth::AuthError {
    cli_framework::auth::AuthError::Provider {
        message: message.to_owned(),
        source: None,
    }
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

#[cfg(test)]
mod discovery_security_tests {
    use super::*;
    use wiremock::{
        matchers::{method, path},
        Mock, MockServer, ResponseTemplate,
    };

    fn test_builder(issuer: &str, cache_dir: &std::path::Path) -> OidcClientBuilder {
        OidcClient::builder()
            .issuer_url(issuer)
            .client_id("native-test")
            .flow(OidcFlow::DeviceCode)
            .cache_dir(cache_dir.to_owned())
    }

    #[test]
    fn builder_rejects_issuer_components_normalization_would_discard() {
        let cache = tempfile::tempdir().unwrap();
        for issuer in [
            "https://user@issuer.example/realm",
            "https://user:password@issuer.example/realm",
            "https://issuer.example/realm?tenant=one",
            "https://issuer.example/realm#fragment",
            "http://issuer.example/realm",
        ] {
            let message = match test_builder(issuer, cache.path()).build() {
                Ok(_) => panic!("unsafe issuer was accepted"),
                Err(error) => error.to_string(),
            };
            assert!(!message.contains(issuer));
            assert!(!message.contains("password"));
        }
        assert!(test_builder("https://issuer.example/realm", cache.path())
            .build()
            .is_ok());
    }

    #[tokio::test]
    async fn discovery_accepts_secure_query_endpoints_and_requires_token_endpoint() {
        let server = MockServer::start().await;
        let http = make_http_client().unwrap();
        let metadata = serde_json::json!({
            "issuer": server.uri(),
            "token_endpoint": format!("{}/token?tenant=one", server.uri()),
            "authorization_endpoint": format!("{}/authorize?tenant=one", server.uri()),
            "device_authorization_endpoint": format!("{}/device?tenant=one", server.uri()),
        });
        Mock::given(method("GET"))
            .and(path("/.well-known/openid-configuration"))
            .respond_with(ResponseTemplate::new(200).set_body_json(metadata))
            .mount(&server)
            .await;

        let discovered = fetch_discovery(&server.uri(), &http).await.unwrap();
        assert!(discovered.token_endpoint.ends_with("/token?tenant=one"));
        assert!(discovered
            .authorization_endpoint
            .unwrap()
            .ends_with("/authorize?tenant=one"));
        assert!(discovered
            .device_authorization_endpoint
            .unwrap()
            .ends_with("/device?tenant=one"));

        let missing = serde_json::json!({"issuer": server.uri()});
        let error = required_discovered_endpoint(&missing, "token_endpoint").unwrap_err();
        assert!(error.to_string().contains("omitted a required endpoint"));
    }

    #[test]
    fn discovery_endpoint_validation_fails_closed_without_reflecting_urls() {
        for endpoint in [
            "http://remote.example/token",
            "https://user:secret@issuer.example/token",
            "https://issuer.example/token#secret",
            "file:///tmp/token",
        ] {
            let document = serde_json::json!({"token_endpoint": endpoint});
            let message = required_discovered_endpoint(&document, "token_endpoint")
                .unwrap_err()
                .to_string();
            assert_eq!(
                message,
                "authentication provider error: OIDC discovery advertised an unsafe endpoint"
            );
            assert!(!message.contains(endpoint));
        }
    }

    #[tokio::test]
    async fn discovery_does_not_follow_redirects_or_parse_error_bodies() {
        let destination = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "issuer": destination.uri(),
                "token_endpoint": format!("{}/token", destination.uri()),
            })))
            .expect(0)
            .mount(&destination)
            .await;

        let issuer = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/.well-known/openid-configuration"))
            .respond_with(
                ResponseTemplate::new(307)
                    .insert_header("Location", format!("{}/redirected", destination.uri())),
            )
            .mount(&issuer)
            .await;

        let error = match fetch_discovery(&issuer.uri(), &make_http_client().unwrap()).await {
            Ok(_) => panic!("redirected discovery must fail closed"),
            Err(error) => error,
        };
        assert_eq!(
            error.to_string(),
            "authentication provider error: OIDC discovery returned a non-success status"
        );
        destination.verify().await;
    }

    #[tokio::test]
    async fn credential_posts_are_never_replayed_across_redirects() {
        for redirect_status in [307, 308] {
            let destination = MockServer::start().await;
            Mock::given(method("POST"))
                .respond_with(ResponseTemplate::new(200))
                .expect(0)
                .mount(&destination)
                .await;

            let issuer = MockServer::start().await;
            Mock::given(method("POST"))
                .and(path("/token"))
                .respond_with(
                    ResponseTemplate::new(redirect_status)
                        .insert_header("Location", format!("{}/capture", destination.uri())),
                )
                .expect(1)
                .mount(&issuer)
                .await;

            let response = make_http_client()
                .unwrap()
                .post(format!("{}/token", issuer.uri()))
                .form(&[
                    ("grant_type", "refresh_token"),
                    ("refresh_token", "fixture-secret-refresh-token"),
                ])
                .send()
                .await
                .unwrap();
            assert_eq!(response.status().as_u16(), redirect_status);
            issuer.verify().await;
            destination.verify().await;
        }
    }
}

#[cfg(test)]
mod flow_contract_tests {
    use super::*;
    use cli_framework::auth::AuthFlowReporter;
    use cli_framework::secrets::InMemorySecretStore;
    use std::sync::Arc;
    use wiremock::{
        matchers::{method, path},
        Mock, MockServer, ResponseTemplate,
    };

    struct SilentReporter;

    impl AuthFlowReporter for SilentReporter {
        fn user_code(&self, _verification_uri: &str, _user_code: &str) {}
        fn message(&self, _line: &str) {}
    }

    fn client(issuer: &str, flow: OidcFlow, cache: &std::path::Path) -> OidcClient {
        OidcClient::builder()
            .issuer_url(issuer)
            .client_id("contract-client")
            .flow(flow)
            .cache_dir(cache.to_owned())
            .secret_store(Arc::new(InMemorySecretStore::new()))
            .reporter(Arc::new(SilentReporter))
            .open_browser(false)
            .build()
            .unwrap()
    }

    fn discovery(server: &MockServer) -> DiscoveryDoc {
        DiscoveryDoc {
            token_endpoint: format!("{}/token", server.uri()),
            device_authorization_endpoint: Some(format!("{}/device", server.uri())),
            authorization_endpoint: Some(format!("{}/authorize", server.uri())),
        }
    }

    #[tokio::test]
    async fn refresh_redeems_and_returns_the_new_cached_token() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/token"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "access_token": "refreshed",
                "refresh_token": "next-refresh",
                "token_type": "Bearer",
                "expires_in": 3600,
            })))
            .mount(&server)
            .await;
        let cache = tempfile::tempdir().unwrap();
        let client = client(&server.uri(), OidcFlow::DeviceCode, cache.path());

        let token = client
            .do_refresh(&discovery(&server), "old-refresh")
            .await
            .unwrap();
        assert_eq!(token.as_bearer(), "refreshed");
    }

    #[tokio::test]
    async fn interactive_client_credentials_uses_the_configured_grant() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/token"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "access_token": "service-token",
                "token_type": "Bearer",
                "expires_in": 3600,
            })))
            .mount(&server)
            .await;
        let cache = tempfile::tempdir().unwrap();
        let client = client(
            &server.uri(),
            OidcFlow::ClientCredentials {
                client_secret: SecretString::new("secret".to_owned()),
                token_auth: TokenAuthMethod::Post,
            },
            cache.path(),
        );
        let _ = client
            .discovery
            .get_or_init(|| async { discovery(&server) })
            .await;

        client.do_interactive_login().await.unwrap();
    }

    async fn device_result(body: serde_json::Value) -> String {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/device"))
            .respond_with(ResponseTemplate::new(200).set_body_json(body))
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/token"))
            .respond_with(ResponseTemplate::new(400).set_body_json(serde_json::json!({
                "error": "access_denied",
                "error_description": "operator declined",
            })))
            .mount(&server)
            .await;
        let cache = tempfile::tempdir().unwrap();
        client(&server.uri(), OidcFlow::DeviceCode, cache.path())
            .do_device_code_login(&discovery(&server))
            .await
            .unwrap_err()
            .to_string()
    }

    #[tokio::test]
    async fn device_response_contract_rejects_errors_missing_fields_and_expiry() {
        assert!(
            device_result(serde_json::json!({"error": "invalid_request"}))
                .await
                .contains("invalid_request")
        );
        for (body, field) in [
            (serde_json::json!({}), "verification_uri"),
            (
                serde_json::json!({"verification_uri": "https://verify.example"}),
                "user_code",
            ),
            (
                serde_json::json!({
                    "verification_uri": "https://verify.example",
                    "user_code": "CODE"
                }),
                "device_code",
            ),
        ] {
            assert!(device_result(body).await.contains(field));
        }
        assert!(device_result(serde_json::json!({
            "verification_uri": "https://verify.example",
            "user_code": "CODE",
            "device_code": "device",
            "expires_in": 0,
            "interval": 0,
        }))
        .await
        .contains("expired"));
        assert!(device_result(serde_json::json!({
            "verification_uri": "https://verify.example",
            "user_code": "CODE",
            "device_code": "device",
            "expires_in": 5,
            "interval": 0,
        }))
        .await
        .contains("access_denied"));
    }

    #[tokio::test]
    async fn flow_preconditions_fail_before_browser_or_credential_networking() {
        let server = MockServer::start().await;
        let cache = tempfile::tempdir().unwrap();
        let device = client(&server.uri(), OidcFlow::DeviceCode, cache.path());
        let missing_device = DiscoveryDoc {
            token_endpoint: format!("{}/token", server.uri()),
            device_authorization_endpoint: None,
            authorization_endpoint: None,
        };
        assert!(device
            .do_device_code_login(&missing_device)
            .await
            .unwrap_err()
            .to_string()
            .contains("device_authorization_endpoint"));

        let pkce = client(
            &server.uri(),
            OidcFlow::AuthCodePkce {
                redirect: RedirectConfig {
                    port: RedirectPort::Ephemeral,
                },
            },
            cache.path(),
        );
        assert!(pkce
            .do_auth_code_pkce_login(
                &missing_device,
                &RedirectConfig {
                    port: RedirectPort::Ephemeral,
                }
            )
            .await
            .unwrap_err()
            .to_string()
            .contains("authorization_endpoint"));
        let malformed_authorization = DiscoveryDoc {
            authorization_endpoint: Some("not an absolute URL".to_owned()),
            ..missing_device
        };
        assert!(pkce
            .do_auth_code_pkce_login(
                &malformed_authorization,
                &RedirectConfig {
                    port: RedirectPort::Ephemeral,
                }
            )
            .await
            .is_err());
    }

    #[tokio::test]
    async fn token_response_requires_access_token_and_bearer_type() {
        let cache = tempfile::tempdir().unwrap();
        let client = client("https://issuer.example", OidcFlow::DeviceCode, cache.path());
        assert!(client
            .store_token_response(&serde_json::json!({"token_type": "Bearer"}))
            .await
            .unwrap_err()
            .to_string()
            .contains("missing access_token"));
        assert!(client
            .store_token_response(&serde_json::json!({
                "access_token": "opaque",
                "token_type": "DPoP"
            }))
            .await
            .unwrap_err()
            .to_string()
            .contains("expected token_type=Bearer"));
    }

    #[test]
    fn environment_flow_contract_covers_explicit_values_and_scopes() {
        let pkce = "CFW_TEST_OIDC_PKCE_FLOW";
        std::env::set_var(format!("{pkce}_ISSUER_URL"), "https://issuer.example");
        std::env::set_var(format!("{pkce}_CLIENT_ID"), "native");
        std::env::set_var(format!("{pkce}_FLOW"), "pkce");
        std::env::set_var(format!("{pkce}_SCOPES"), "openid, profile email");
        let builder = OidcClientBuilder::from_env(pkce).unwrap();
        assert_eq!(builder.scopes.unwrap().len(), 3);

        let missing_secret = "CFW_TEST_OIDC_CC_MISSING_SECRET";
        std::env::set_var(
            format!("{missing_secret}_ISSUER_URL"),
            "https://issuer.example",
        );
        std::env::set_var(format!("{missing_secret}_CLIENT_ID"), "native");
        std::env::set_var(format!("{missing_secret}_FLOW"), "client-credentials");
        assert!(OidcClientBuilder::from_env(missing_secret).is_err());

        let unknown = "CFW_TEST_OIDC_UNKNOWN_FLOW";
        std::env::set_var(format!("{unknown}_ISSUER_URL"), "https://issuer.example");
        std::env::set_var(format!("{unknown}_CLIENT_ID"), "native");
        std::env::set_var(format!("{unknown}_FLOW"), "surprise");
        assert!(OidcClientBuilder::from_env(unknown).is_err());

        assert!(matches!(
            OidcFlow::auto_interactive(),
            OidcFlow::DeviceCode | OidcFlow::AuthCodePkce { .. }
        ));
    }
}

#[cfg(test)]
mod token_cache_tests {
    use super::*;
    use cli_framework::auth::TokenProvider;
    use cli_framework::secrets::{InMemorySecretStore, SecretError, SecretValue};

    struct FailPutStore;

    #[async_trait]
    impl SecretStore for FailPutStore {
        async fn get(&self, _key: &SecretKey) -> Result<SecretValue, SecretError> {
            Err(SecretError::NotFound)
        }
        async fn put(&self, _key: &SecretKey, _value: SecretValue) -> Result<(), SecretError> {
            Err(SecretError::backend("blob too large"))
        }
        async fn delete(&self, _key: &SecretKey) -> Result<(), SecretError> {
            Ok(())
        }
        async fn rotate(&self, _key: &SecretKey) -> Result<SecretValue, SecretError> {
            Err(SecretError::NotSupported("rotate"))
        }
    }

    fn pkce_client(dir: &std::path::Path, store: Arc<dyn SecretStore>) -> OidcClient {
        OidcClient::builder()
            .issuer_url("https://auth.example.com")
            .client_id("aidesktop")
            .flow(OidcFlow::AuthCodePkce {
                redirect: RedirectConfig::default(),
            })
            .cache_dir(dir.to_path_buf())
            .secret_store(store)
            .refresh_skew(Duration::from_secs(60))
            .build()
            .expect("build")
    }

    #[tokio::test]
    async fn pkce_token_returns_unexpired_access_inside_refresh_skew() {
        let dir = tempfile::TempDir::new().unwrap();
        let store: Arc<dyn SecretStore> = Arc::new(InMemorySecretStore::new());
        let client = pkce_client(dir.path(), store);
        client
            .store_token_response(&serde_json::json!({
                "access_token": "just-issued",
                "token_type": "Bearer",
                "expires_in": 45u64,
            }))
            .await
            .expect("store");

        let token = client
            .token()
            .await
            .expect("just-issued PKCE token must not become NotAuthenticated");
        assert_eq!(token.as_bearer(), "just-issued");
    }

    #[tokio::test]
    async fn pkce_token_is_not_authenticated_when_access_has_expired() {
        let dir = tempfile::TempDir::new().unwrap();
        let store: Arc<dyn SecretStore> = Arc::new(InMemorySecretStore::new());
        let client = pkce_client(dir.path(), store);
        client
            .store_token_response(&serde_json::json!({
                "access_token": "stale",
                "token_type": "Bearer",
                "expires_in": 0u64,
            }))
            .await
            .expect("store");

        let err = client.token().await.expect_err("expired PKCE token");
        assert!(
            matches!(err, cli_framework::auth::AuthError::NotAuthenticated),
            "got {err}"
        );
    }

    #[tokio::test]
    async fn store_token_response_fails_when_cache_write_fails() {
        let dir = tempfile::TempDir::new().unwrap();
        let client = pkce_client(dir.path(), Arc::new(FailPutStore));
        let err = client
            .store_token_response(&serde_json::json!({
                "access_token": "x",
                "token_type": "Bearer",
                "expires_in": 3600u64,
            }))
            .await
            .expect_err("write failure must fail login, not succeed silently");
        let msg = err.to_string();
        assert!(
            msg.contains("token cache write failed"),
            "expected cache-write error, got {msg}"
        );
    }
}
