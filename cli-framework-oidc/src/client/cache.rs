//! `OidcClient`'s token cache, stored through a `SecretStore`.
//!
//! The entire [`CacheFile`] (every cached grant, keyed by [`OidcClient::cache_key`])
//! is serialized as one JSON blob and stored as a single [`SecretValue`] under
//! a [`SecretKey`] (see [`default_cache_secret_key`]) — one file/secret per
//! `SecretStore` (typically per `cache_dir`), not one per grant.
//!
//! Default key: `<app>/oidc/token.json`, derived from the builder `app_name`.
//! When the default [`EnvFileSecretStore`] backend is in play, that key is the
//! relative path under `cache_dir` (for example
//! `<cache_dir>/aidesktop/oidc/token.json`).
//!
//! A legacy flat key `"oidc-token.json"` is still read. The first successful
//! write to the namespaced key migrates by deleting the legacy entry.
//!
//! When a non-file backend (e.g. `secrets-openbao::OpenBaoSecretStore`) is
//! injected instead, no plaintext token file is ever written — the bytes go
//! straight through that backend's `put`/`get`.

use cli_framework::secrets::{SecretError, SecretKey, SecretStore, SecretValue};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
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

/// Historical cache key used before namespaced `<app>/oidc/token.json`.
pub fn legacy_cache_secret_key() -> SecretKey {
    SecretKey::new(["oidc-token.json"]).expect("static key is valid")
}

/// Default cache key: `<app>/oidc/token.json`.
///
/// `app_name` is sanitized to [`SecretKey`] charset; empty or reserved
/// names become `"default"`.
pub fn default_cache_secret_key(app_name: Option<&str>) -> SecretKey {
    let app = sanitize_app_segment(app_name.unwrap_or("default"));
    SecretKey::new([app.as_str(), "oidc", "token.json"]).expect("sanitized key is valid")
}

fn sanitize_app_segment(name: &str) -> String {
    let s: String = name
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '_' || c == '-' || c == '.' {
                c
            } else {
                '-'
            }
        })
        .collect();
    if s.is_empty() || s == "." || s == ".." {
        "default".to_string()
    } else {
        s
    }
}

/// Lock-file path relative to `cache_dir`, sibling of the cache file:
/// last segment `token.json` → `token.lock`.
pub fn cache_lock_relpath(key: &SecretKey) -> PathBuf {
    let segs: Vec<&str> = key.segments().collect();
    let mut p = PathBuf::new();
    let last = segs.len().saturating_sub(1);
    for (i, seg) in segs.into_iter().enumerate() {
        if i == last {
            let lock = match seg.strip_suffix(".json") {
                Some(stem) => format!("{stem}.lock"),
                None => format!("{seg}.lock"),
            };
            p.push(lock);
        } else {
            p.push(seg);
        }
    }
    p
}

async fn load_at(store: &dyn SecretStore, key: &SecretKey) -> Result<CacheFile, SecretError> {
    let value = store.get(key).await?;
    match value
        .expose_str()
        .ok()
        .and_then(|s| serde_json::from_str(s).ok())
    {
        Some(cache) => Ok(cache),
        None => {
            tracing::warn!("oidc token cache: parse error, treating as empty");
            Ok(CacheFile::empty())
        }
    }
}

pub async fn read_cache(store: &dyn SecretStore, key: &SecretKey) -> CacheFile {
    match load_at(store, key).await {
        Ok(cache) => cache,
        Err(SecretError::NotFound) => {
            let legacy = legacy_cache_secret_key();
            if key == &legacy {
                return CacheFile::empty();
            }
            match load_at(store, &legacy).await {
                Ok(cache) => cache,
                Err(SecretError::NotFound) => CacheFile::empty(),
                Err(e) => {
                    tracing::warn!("oidc token cache: legacy read failed ({e}), treating as empty");
                    CacheFile::empty()
                }
            }
        }
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
        .map_err(|e| anyhow::anyhow!(e))?;
    let legacy = legacy_cache_secret_key();
    if key != &legacy {
        if let Err(e) = store.delete(&legacy).await {
            tracing::warn!("oidc token cache: legacy delete failed: {e}");
        }
    }
    Ok(())
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_key_uses_app_name() {
        assert_eq!(
            default_cache_secret_key(Some("aidesktop")).as_str(),
            "aidesktop/oidc/token.json"
        );
    }

    #[test]
    fn default_key_without_app_name_is_default_namespace() {
        assert_eq!(
            default_cache_secret_key(None).as_str(),
            "default/oidc/token.json"
        );
    }

    #[test]
    fn sanitizes_invalid_app_name_chars() {
        assert_eq!(
            default_cache_secret_key(Some("my app")).as_str(),
            "my-app/oidc/token.json"
        );
        assert_eq!(
            default_cache_secret_key(Some("")).as_str(),
            "default/oidc/token.json"
        );
        assert_eq!(
            default_cache_secret_key(Some("..")).as_str(),
            "default/oidc/token.json"
        );
    }

    #[test]
    fn lock_relpath_is_sibling_of_cache_file() {
        let key = default_cache_secret_key(Some("aidesktop"));
        assert_eq!(
            cache_lock_relpath(&key),
            PathBuf::from("aidesktop/oidc/token.lock")
        );
        assert_eq!(
            cache_lock_relpath(&legacy_cache_secret_key()),
            PathBuf::from("oidc-token.lock")
        );
    }
}
