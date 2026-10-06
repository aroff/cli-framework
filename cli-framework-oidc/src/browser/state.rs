/// Shared runtime state for both the browser session layer and the dual-mode layer.
use crate::jwks::{fetch_discovery, fetch_jwks, filter_keys, JwksCache, KeyResult, OidcDiscovery};
use jsonwebtoken::Algorithm;
use std::time::{Duration, Instant};
use tokio::sync::{Mutex, OnceCell};

use std::collections::HashMap;

const LOGIN_TTL: Duration = Duration::from_secs(600);
const MAX_PENDING_LOGINS: usize = 1024;

pub(crate) struct PendingLogin {
    pub nonce: String,
    deadline: Instant,
}

impl PendingLogin {
    pub fn is_live(&self) -> bool {
        self.deadline > Instant::now()
    }
}

pub(crate) struct BrowserLayerState {
    pub cfg: super::OidcBrowserSessionConfig,
    /// HMAC key derived from session_key (not stored in cfg to keep it separate).
    pub hmac_key: [u8; 32],
    pub pending_logins: Mutex<HashMap<String, PendingLogin>>,
    pub algorithms: Vec<Algorithm>,
    pub jwks_cache: Mutex<JwksCache>,
    pub discovery: OnceCell<OidcDiscovery>,
    pub last_forced_refetch: Mutex<Option<Instant>>,
    pub refetch_gate: Mutex<()>,
    pub http: reqwest::Client,
}

impl BrowserLayerState {
    pub async fn begin_login(&self, state: String) -> Option<String> {
        let mut pending = self.pending_logins.lock().await;
        let now = Instant::now();
        pending.retain(|_, login| login.deadline > now);
        if pending.len() >= MAX_PENDING_LOGINS || pending.contains_key(&state) {
            return None;
        }
        let nonce = super::auth_state::random_state();
        pending.insert(
            state,
            PendingLogin {
                nonce: nonce.clone(),
                deadline: now + LOGIN_TTL,
            },
        );
        Some(nonce)
    }

    /// Remove before any provider I/O; simultaneous or failed callbacks cannot replay.
    pub async fn consume_login(&self, state: &str) -> Option<PendingLogin> {
        let login = self.pending_logins.lock().await.remove(state)?;
        login.is_live().then_some(login)
    }
    pub async fn authorization_endpoint(&self) -> Result<String, String> {
        self.discovery()
            .await?
            .authorization_endpoint
            .clone()
            .ok_or_else(|| "missing authorization endpoint".into())
    }
    pub async fn token_endpoint(&self) -> String {
        self.discovery()
            .await
            .map(|d| d.token_endpoint.clone())
            .unwrap_or_default()
    }

    pub async fn end_session_endpoint(&self) -> Option<String> {
        self.discovery()
            .await
            .ok()
            .and_then(|d| d.end_session_endpoint.clone())
    }

    async fn discovery(&self) -> Result<&OidcDiscovery, String> {
        self.discovery
            .get_or_try_init(|| fetch_discovery(&self.cfg.issuer_url, &self.http))
            .await
            .map_err(|e| e.to_string())
    }

    pub async fn get_decoding_keys(&self, kid: &Option<String>) -> KeyResult {
        let jwks_ttl = self.cfg.jwks_ttl;
        let min_refetch = Duration::from_secs(60);

        // Fast path
        {
            let cache = self.jwks_cache.lock().await;
            if cache.is_fresh(jwks_ttl) {
                let result = filter_keys(&cache.keys, kid);
                if !matches!(result, KeyResult::UnknownKid) {
                    return result;
                }
            }
        }

        // Single-flight gate
        let _guard = self.refetch_gate.lock().await;
        {
            let cache = self.jwks_cache.lock().await;
            if cache.is_fresh(jwks_ttl) {
                let result = filter_keys(&cache.keys, kid);
                if !matches!(result, KeyResult::UnknownKid) {
                    return result;
                }
            }
        }

        let jwks_uri = match self.get_jwks_uri().await {
            Ok(u) => u,
            Err(e) => {
                tracing::warn!("oidc-browser: failed to get jwks_uri: {e}");
                let cache = self.jwks_cache.lock().await;
                return if cache.is_empty() {
                    KeyResult::Unavailable
                } else {
                    filter_keys(&cache.keys, kid)
                };
            }
        };

        {
            let last = self.last_forced_refetch.lock().await;
            if let Some(t) = *last {
                if t.elapsed() < min_refetch {
                    let cache = self.jwks_cache.lock().await;
                    return if cache.is_empty() {
                        KeyResult::Unavailable
                    } else {
                        filter_keys(&cache.keys, kid)
                    };
                }
            }
        }

        match fetch_jwks(&jwks_uri, &self.http).await {
            Ok(keys) => {
                let mut cache = self.jwks_cache.lock().await;
                cache.keys = keys;
                cache.fetched_at = Some(Instant::now());
                *self.last_forced_refetch.lock().await = Some(Instant::now());
                filter_keys(&cache.keys, kid)
            }
            Err(e) => {
                tracing::warn!("oidc-browser: jwks fetch failed: {e}");
                let cache = self.jwks_cache.lock().await;
                if cache.is_empty() {
                    KeyResult::Unavailable
                } else {
                    filter_keys(&cache.keys, kid)
                }
            }
        }
    }

    async fn get_jwks_uri(&self) -> Result<String, String> {
        if let Some(ref uri) = self.cfg.jwks_uri {
            return Ok(uri.clone());
        }
        let disc = self.discovery().await?;
        Ok(disc.jwks_uri.clone())
    }
}

#[cfg(test)]
mod login_tests {
    use super::*;
    use crate::browser::{
        AudiencePolicy, OidcBrowserSession, OidcBrowserSessionConfig, SessionKey,
    };

    fn session_runtime() -> OidcBrowserSession {
        OidcBrowserSession::new(OidcBrowserSessionConfig::new(
            "https://issuer.example",
            "client",
            "https://app.example/callback",
            SessionKey::from_bytes([7; 32]),
            AudiencePolicy::Require("api".into()),
        ))
        .unwrap()
    }

    #[tokio::test]
    async fn pending_logins_are_bounded_expiring_and_consumed_once() {
        let runtime = session_runtime();
        let state = &runtime.state;
        for index in 0..MAX_PENDING_LOGINS {
            assert!(state.begin_login(index.to_string()).await.is_some());
        }
        assert!(state.begin_login("overflow".into()).await.is_none());
        assert!(state.begin_login("0".into()).await.is_none());
        state
            .pending_logins
            .lock()
            .await
            .get_mut("0")
            .unwrap()
            .deadline = Instant::now();
        assert!(state.consume_login("0").await.is_none());
        assert!(state.begin_login("replacement".into()).await.is_some());
        assert!(state.consume_login("replacement").await.is_some());
        assert!(state.consume_login("replacement").await.is_none());
        assert!(state.consume_login("never-issued").await.is_none());
    }

    #[tokio::test]
    async fn cloned_runtimes_share_pending_state_and_restart_invalidates_it() {
        let runtime = session_runtime();
        assert!(runtime.state.begin_login("issued".into()).await.is_some());
        let clone = runtime.clone();
        assert!(clone.state.consume_login("issued").await.is_some());
        assert!(runtime.state.consume_login("issued").await.is_none());
        assert!(runtime
            .state
            .begin_login("before-restart".into())
            .await
            .is_some());
        let restarted = session_runtime();
        assert!(restarted
            .state
            .consume_login("before-restart")
            .await
            .is_none());
    }
}
