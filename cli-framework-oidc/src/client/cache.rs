//! On-disk token cache for OidcClient.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;
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

pub fn read_cache(cache_dir: &Path) -> CacheFile {
    let path = cache_dir.join("oidc-token.json");
    match std::fs::read_to_string(&path) {
        Ok(s) => serde_json::from_str(&s).unwrap_or_else(|_| {
            tracing::warn!("oidc token cache: parse error, treating as empty");
            CacheFile::empty()
        }),
        Err(_) => CacheFile::empty(),
    }
}

pub fn write_cache(cache_dir: &Path, cache: &CacheFile) -> anyhow::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        std::fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(cache_dir)?;
    }
    #[cfg(not(unix))]
    {
        std::fs::create_dir_all(cache_dir)?;
    }

    let data = serde_json::to_string_pretty(cache)?;
    let tmp_path = cache_dir.join(format!("oidc-token.json.tmp.{}", std::process::id()));

    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        let mut f = std::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o600)
            .open(&tmp_path)?;
        use std::io::Write;
        f.write_all(data.as_bytes())?;
        f.sync_all()?;
    }
    #[cfg(not(unix))]
    {
        let mut f = std::fs::File::create(&tmp_path)?;
        use std::io::Write;
        f.write_all(data.as_bytes())?;
        f.sync_all()?;
    }

    std::fs::rename(&tmp_path, cache_dir.join("oidc-token.json"))?;
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
