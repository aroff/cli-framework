//! OIDC client: `OidcClient` implementing `cli_framework::auth::TokenProvider`.

use crate::OidcConfigError;
use async_trait::async_trait;
use secrecy::SecretString;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, SystemTime};

mod cache;
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
}

impl OidcClient {
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

    async fn get_or_refresh_token(
        &self,
    ) -> Result<cli_framework::auth::AccessToken, cli_framework::auth::AuthError> {
        let key = self.cache_key();
        let cache_dir = self.cache_dir.clone();
        let refresh_skew = self.refresh_skew;

        let (access_token, refresh_token, expires_at) = {
            let key = key.clone();
            let cache_dir = cache_dir.clone();
            tokio::task::spawn_blocking(move || {
                let cache = read_cache(&cache_dir);
                let entry = cache.entries.get(&key).cloned()?;
                let exp = entry.expires_at.as_deref().and_then(cache::parse_rfc3339);
                Some((entry.access_token, entry.refresh_token, exp))
            })
            .await
            .map_err(|e| cli_framework::auth::AuthError::Provider {
                message: e.to_string(),
                source: None,
            })?
            .unwrap_or((None, None, None))
        };

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
                let (at, _, exp) = {
                    let key = key.clone();
                    let cache_dir = cache_dir.clone();
                    tokio::task::spawn_blocking(move || {
                        let cache = read_cache(&cache_dir);
                        let entry = cache.entries.get(&key).cloned()?;
                        let exp = entry.expires_at.as_deref().and_then(cache::parse_rfc3339);
                        Some((entry.access_token, entry.refresh_token, exp))
                    })
                    .await
                    .map_err(|e| cli_framework::auth::AuthError::Provider {
                        message: e.to_string(),
                        source: None,
                    })?
                    .unwrap_or((None, None, None))
                };
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
        let cache_dir = self.cache_dir.clone();
        let (at, _, exp) = tokio::task::spawn_blocking(move || {
            let cache = read_cache(&cache_dir);
            let entry = cache.entries.get(&key).cloned()?;
            let exp = entry.expires_at.as_deref().and_then(cache::parse_rfc3339);
            Some((entry.access_token, entry.refresh_token, exp))
        })
        .await
        .map_err(|e| cli_framework::auth::AuthError::Provider {
            message: e.to_string(),
            source: None,
        })?
        .unwrap_or((None, None, None));

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
        let params = [
            ("client_id", self.client_id.as_str()),
            ("scope", scope_str.as_str()),
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

        let verification_uri = resp["verification_uri"].as_str().unwrap_or("");
        let user_code = resp["user_code"].as_str().unwrap_or("");
        let device_code = resp["device_code"].as_str().unwrap_or("").to_string();
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
        use base64::Engine;
        use sha2::{Digest, Sha256};

        // Generate PKCE
        let code_verifier_bytes: Vec<u8> = (0..32).map(|_| rand_byte()).collect();
        let code_verifier =
            base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(&code_verifier_bytes);
        let code_challenge = {
            let hash = Sha256::digest(code_verifier.as_bytes());
            base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(hash)
        };
        let state: String = (0..16).map(|_| format!("{:02x}", rand_byte())).collect();

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
        let cache_dir = self.cache_dir.clone();
        let access_token = access_token.to_string();
        let scopes = self.effective_scopes();

        tokio::task::spawn_blocking(move || {
            use fs2::FileExt;
            std::fs::create_dir_all(&cache_dir).ok();
            let lock_path = cache_dir.join("oidc-token.lock");
            let lock_file = open_lock_file(&lock_path).unwrap();
            lock_file.lock_exclusive().ok();

            let mut cache = read_cache(&cache_dir);
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

            if let Err(e) = write_cache(&cache_dir, &cache) {
                tracing::warn!("oidc token cache: write failed: {e}");
            }
            #[allow(clippy::incompatible_msrv)]
            lock_file.unlock().ok();
        })
        .await
        .map_err(|e| cli_framework::auth::AuthError::Provider {
            message: e.to_string(),
            source: None,
        })
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
        let cache_dir = self.cache_dir.clone();
        let _ = tokio::task::spawn_blocking(move || {
            use fs2::FileExt;
            std::fs::create_dir_all(&cache_dir).ok();
            let lock_path = cache_dir.join("oidc-token.lock");
            let lock_file = open_lock_file(&lock_path).ok()?;
            lock_file.lock_exclusive().ok()?;
            let mut cache = read_cache(&cache_dir);
            if let Some(entry) = cache.entries.get_mut(&key) {
                entry.access_token = None;
                entry.expires_at = None;
            }
            if let Err(e) = write_cache(&cache_dir, &cache) {
                tracing::warn!("oidc token cache: invalidate write failed: {e}");
            }
            #[allow(clippy::incompatible_msrv)]
            lock_file.unlock().ok();
            Some(())
        })
        .await;
    }

    async fn peek(&self) -> Option<cli_framework::auth::TokenStatus> {
        let key = self.cache_key();
        let cache_dir = self.cache_dir.clone();
        tokio::task::spawn_blocking(move || {
            let cache = read_cache(&cache_dir);
            let entry = cache.entries.get(&key).cloned()?;
            let has_access = entry.access_token.is_some();
            let has_refresh = entry.refresh_token.is_some();
            let expires_at = entry.expires_at.as_deref().and_then(cache::parse_rfc3339);
            Some(cli_framework::auth::TokenStatus {
                logged_in: has_access || has_refresh,
                expires_at: if has_access { expires_at } else { None },
            })
        })
        .await
        .ok()
        .flatten()
    }

    async fn login(&self) -> Result<(), cli_framework::auth::AuthError> {
        self.do_interactive_login().await
    }

    async fn logout(&self) -> Result<(), cli_framework::auth::AuthError> {
        let key = self.cache_key();
        let cache_dir = self.cache_dir.clone();
        tokio::task::spawn_blocking(move || {
            use fs2::FileExt;
            std::fs::create_dir_all(&cache_dir).ok();
            let lock_path = cache_dir.join("oidc-token.lock");
            let lock_file = open_lock_file(&lock_path).map_err(|e| {
                cli_framework::auth::AuthError::Provider {
                    message: e.to_string(),
                    source: None,
                }
            })?;
            lock_file.lock_exclusive().ok();
            let mut cache = read_cache(&cache_dir);
            cache.entries.remove(&key);
            if let Err(e) = write_cache(&cache_dir, &cache) {
                tracing::warn!("oidc token cache: logout write failed: {e}");
            }
            #[allow(clippy::incompatible_msrv)]
            lock_file.unlock().ok();
            Ok::<(), cli_framework::auth::AuthError>(())
        })
        .await
        .map_err(|e| cli_framework::auth::AuthError::Provider {
            message: e.to_string(),
            source: None,
        })?
    }
}

// ── OidcClientBuilder ───────────────────────────────────────────────────────

pub struct OidcClientBuilder {
    issuer_url: Option<String>,
    client_id: Option<String>,
    flow: Option<OidcFlow>,
    scopes: Option<Vec<String>>,
    cache_dir: Option<PathBuf>,
    reporter: Option<Arc<dyn cli_framework::auth::AuthFlowReporter>>,
    refresh_skew: Duration,
}

impl OidcClientBuilder {
    fn new() -> Self {
        Self {
            issuer_url: None,
            client_id: None,
            flow: None,
            scopes: None,
            cache_dir: None,
            reporter: None,
            refresh_skew: Duration::from_secs(60),
        }
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

    pub fn reporter(mut self, r: Arc<dyn cli_framework::auth::AuthFlowReporter>) -> Self {
        self.reporter = Some(r);
        self
    }

    pub fn refresh_skew(mut self, d: Duration) -> Self {
        self.refresh_skew = d;
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
        let cache_dir = self
            .cache_dir
            .ok_or(OidcConfigError::MissingField("cache_dir"))?;
        let reporter = self
            .reporter
            .unwrap_or_else(|| Arc::new(cli_framework::auth::StderrAuthFlowReporter));

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
        })
    }
}

// ── Helpers ─────────────────────────────────────────────────────────────────

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

fn rand_byte() -> u8 {
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::{SystemTime, UNIX_EPOCH};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let t = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos() as u64;
    let c = COUNTER.fetch_add(1, Ordering::Relaxed);
    let mixed = t
        .wrapping_mul(6364136223846793005)
        .wrapping_add(c.wrapping_mul(1442695040888963407));
    (mixed >> 33) as u8
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
