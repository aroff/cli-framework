//! [`PolicyCache`]: the on-disk store of the last-received [`Policy`].
//!
//! Spec 021, "Cache": stored verbatim with its ETag and fetch time, under the
//! platform **data** directory — not the config directory, because it is
//! derived state, not user-authored. Reuses [`ConfigBackend`] (the same
//! storage abstraction the prior config slice uses for user-authored
//! documents) purely as a byte-level seam; the versioning/migration
//! machinery in [`crate::config::ConfigStore`] does not apply here, since the
//! cache document has no schema evolution story of its own — it is always
//! the verbatim body of whatever a real server most recently sent.

use crate::config::{ConfigBackend, ConfigError, FileBackend};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

/// The cached policy body (verbatim JSON, so unknown/newer fields the local
/// [`Policy`](crate::config::Policy) type doesn't know about still survive a
/// read-modify-write cycle), its ETag, and when it was last confirmed fresh.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PolicyCacheEntry {
    pub policy: Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub etag: Option<String>,
    pub fetched_at_epoch_secs: u64,
}

/// The current wall-clock time as epoch seconds — used to stamp
/// [`PolicyCacheEntry::fetched_at_epoch_secs`] and to evaluate staleness.
/// `std::time::UNIX_EPOCH` is always in the past for any real clock; the
/// `unwrap_or` branch exists only so a maliciously-set system clock can never
/// panic this crate.
pub fn now_epoch_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

/// The on-disk store for one application's [`PolicyCacheEntry`].
pub struct PolicyCache {
    backend: Arc<dyn ConfigBackend>,
}

impl PolicyCache {
    /// Build a cache over an arbitrary [`ConfigBackend`] — used by tests with
    /// [`crate::config::InMemoryBackend`].
    pub fn new(backend: Arc<dyn ConfigBackend>) -> Self {
        Self { backend }
    }

    /// The default location: `<platform-data-dir>/<app_name>/policy-cache.json`.
    ///
    /// Returns [`ConfigError::Io`] if the platform has no resolvable data
    /// directory, mirroring [`FileBackend::for_app`]'s treatment of the
    /// config directory.
    pub fn for_app(app_name: &str) -> Result<Self, ConfigError> {
        let base = dirs::data_dir().ok_or_else(|| ConfigError::Io {
            path: std::path::PathBuf::from(app_name),
            source: std::io::Error::new(
                std::io::ErrorKind::NotFound,
                "no platform data directory available",
            ),
        })?;
        let path = base.join(app_name).join("policy-cache.json");
        Ok(Self::new(Arc::new(FileBackend::new(path))))
    }

    /// The current cache entry, or `None` if nothing has been cached yet (an
    /// empty backend — first run, or a backend just [`Self::clear`]ed).
    pub fn read(&self) -> Result<Option<PolicyCacheEntry>, ConfigError> {
        let bytes = self.backend.read()?;
        if bytes.is_empty() {
            return Ok(None);
        }
        let entry: PolicyCacheEntry =
            serde_json::from_slice(&bytes).map_err(|e| ConfigError::Parse {
                backend: self.backend.label(),
                source: Box::new(e),
            })?;
        Ok(Some(entry))
    }

    /// Overwrite the cache with `entry`.
    pub fn write(&self, entry: &PolicyCacheEntry) -> Result<(), ConfigError> {
        let bytes = serde_json::to_vec_pretty(entry).map_err(|e| ConfigError::Serialize {
            backend: self.backend.label(),
            source: Box::new(e),
        })?;
        self.backend.write(&bytes)
    }

    /// Remove any cached entry (spec 021: a 404 "not managed" response must
    /// clear the cache — an application removed from management must not
    /// keep enforcing old rules).
    pub fn clear(&self) -> Result<(), ConfigError> {
        self.backend.write(&[])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::InMemoryBackend;
    use serde_json::json;

    fn entry() -> PolicyCacheEntry {
        PolicyCacheEntry {
            policy: json!({"contract_version": 1}),
            etag: Some("\"abc\"".to_string()),
            fetched_at_epoch_secs: 1000,
        }
    }

    #[test]
    fn read_on_empty_backend_returns_none() {
        let cache = PolicyCache::new(Arc::new(InMemoryBackend::new()));
        assert!(cache.read().unwrap().is_none());
    }

    #[test]
    fn write_then_read_round_trips() {
        let cache = PolicyCache::new(Arc::new(InMemoryBackend::new()));
        cache.write(&entry()).unwrap();
        let back = cache.read().unwrap().unwrap();
        assert_eq!(back, entry());
    }

    #[test]
    fn clear_makes_subsequent_read_return_none() {
        let cache = PolicyCache::new(Arc::new(InMemoryBackend::new()));
        cache.write(&entry()).unwrap();
        cache.clear().unwrap();
        assert!(cache.read().unwrap().is_none());
    }

    #[test]
    fn corrupt_bytes_surface_as_parse_error() {
        let backend = Arc::new(InMemoryBackend::with_bytes(b"not json".to_vec()));
        let cache = PolicyCache::new(backend);
        let err = cache.read().unwrap_err();
        assert!(matches!(err, ConfigError::Parse { .. }));
    }

    // `for_app` resolves under the platform *data* directory, redirected here
    // via `$XDG_DATA_HOME` (mirroring `FileBackend::for_app`'s own test in
    // `tests/unit/config_backend_file.rs`, which redirects `$XDG_CONFIG_HOME`)
    // so this test never touches the real user profile.
    //
    // Note: the `ConfigError::Io` branch (no resolvable data directory at
    // all) is not exercised here for the same reason documented at that
    // test: `dirs::data_dir()` on Linux falls back to a `getpwuid`-style OS
    // user-database lookup when `$HOME`/`$XDG_DATA_HOME` are both unset, so a
    // real user account can't be made to hit that branch portably.
    #[test]
    fn for_app_resolves_under_redirected_data_dir() {
        let dir = tempfile::TempDir::new().unwrap();
        let original = std::env::var("XDG_DATA_HOME").ok();
        std::env::set_var("XDG_DATA_HOME", dir.path());

        let cache = PolicyCache::for_app("my-test-app").unwrap();

        match original {
            Some(v) => std::env::set_var("XDG_DATA_HOME", v),
            None => std::env::remove_var("XDG_DATA_HOME"),
        }

        cache.write(&entry()).unwrap();
        let back = cache.read().unwrap().unwrap();
        assert_eq!(back, entry());
    }

    #[test]
    fn now_epoch_secs_is_plausibly_recent() {
        // Sanity bound: any time after 2020-01-01 (1577836800) and not
        // absurdly far in the future.
        let now = now_epoch_secs();
        assert!(now > 1_577_836_800);
        assert!(now < 4_102_444_800); // before 2100
    }
}
