//! `OidcClient`'s token cache, stored through a `SecretStore`.
//!
//! The entire [`CacheFile`] (every cached grant, keyed by [`OidcClient::cache_key`])
//! is serialized as one JSON blob and stored as a single [`SecretValue`] under
//! a stable [`SecretKey`] (see [`cache_secret_key`]) — one file/secret per
//! `SecretStore` (typically per `cache_dir`), not one per grant. That key was
//! chosen to be `"oidc-token.json"`: when the default [`EnvFileSecretStore`]
//! backend is in play, this reproduces the exact on-disk filename
//! `cli-framework-oidc` has always used, so the pre-existing on-disk cache
//! layout, permissions, and location are unchanged for callers who don't
//! inject a different `SecretStore`.
//!
//! When a non-file backend (e.g. `secrets-openbao::OpenBaoSecretStore`) is
//! injected instead, no plaintext token file is ever written — the bytes go
//! straight through that backend's `put`/`get`.

use cli_framework::secrets::{SecretError, SecretKey, SecretStore, SecretValue};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::time::SystemTime;
use time::{format_description::well_known::Rfc3339, OffsetDateTime};

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct CacheFile {
    pub version: u32,
    pub entries: HashMap<String, CacheEntry>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct CacheEntry {
    pub access_token: Option<String>,
    pub refresh_token: Option<String>,
    pub expires_at: Option<String>, // RFC3339 UTC
    pub obtained_at: String,        // RFC3339 UTC
    pub scopes: Vec<String>,
}

impl CacheFile {
    pub fn empty() -> Self {
        Self {
            version: 1,
            entries: HashMap::new(),
        }
    }
}

/// The stable `SecretStore` key the whole cache file is stored under.
///
/// Fixed at `"oidc-token.json"` so the default [`EnvFileSecretStore`] backend
/// reproduces the historical on-disk filename exactly (see module docs).
pub fn cache_secret_key() -> SecretKey {
    SecretKey::new(["oidc-token.json"]).expect("static key is valid")
}

pub async fn read_cache(store: &dyn SecretStore, key: &SecretKey) -> CacheFile {
    match store.get(key).await {
        Ok(value) => match value
            .expose_str()
            .ok()
            .and_then(|s| serde_json::from_str(s).ok())
        {
            Some(cache) => cache,
            None => {
                tracing::warn!("oidc token cache: parse error, treating as empty");
                CacheFile::empty()
            }
        },
        Err(SecretError::NotFound) => CacheFile::empty(),
        Err(e) => {
            tracing::warn!("oidc token cache: read failed ({e}), treating as empty");
            CacheFile::empty()
        }
    }
}

pub async fn write_cache(
    store: &dyn SecretStore,
    key: &SecretKey,
    cache: &CacheFile,
) -> anyhow::Result<()> {
    let data = serde_json::to_string_pretty(cache)?;
    store
        .put(key, SecretValue::from(data))
        .await
        .map_err(|e| anyhow::anyhow!(e))
}

pub fn format_rfc3339(t: SystemTime) -> String {
    let odt = OffsetDateTime::from(t);
    odt.format(&Rfc3339)
        .unwrap_or_else(|_| "1970-01-01T00:00:00Z".to_string())
}

pub fn parse_rfc3339(s: &str) -> Option<SystemTime> {
    OffsetDateTime::parse(s, &Rfc3339)
        .ok()
        .map(SystemTime::from)
}
