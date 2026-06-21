/// Shared runtime state for both the browser session layer and the dual-mode layer.
use crate::jwks::{fetch_discovery, fetch_jwks, filter_keys, JwksCache, KeyResult, OidcDiscovery};
use jsonwebtoken::Algorithm;
use std::time::{Duration, Instant};
use tokio::sync::{Mutex, OnceCell};

use crate::types::AudiencePolicy;

pub(crate) struct BrowserLayerState {
    pub cfg: super::OidcBrowserSessionConfig,
    /// HMAC key derived from session_key (not stored in cfg to keep it separate).
    pub hmac_key: [u8; 32],
    /// Audience policy used by the dual-mode API layer (may differ from cfg.audience).
    pub api_audience: AudiencePolicy,
    pub algorithms: Vec<Algorithm>,
    pub jwks_cache: Mutex<JwksCache>,
    pub discovery: OnceCell<OidcDiscovery>,
    pub last_forced_refetch: Mutex<Option<Instant>>,
    pub refetch_gate: Mutex<()>,
    pub http: reqwest::Client,
}

impl BrowserLayerState {
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
