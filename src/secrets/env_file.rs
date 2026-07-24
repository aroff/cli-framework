//! [`EnvFileSecretStore`]: the zero-config dev/default backend.

use super::{SecretError, SecretKey, SecretStore, SecretValue};
use async_trait::async_trait;
use std::path::{Path, PathBuf};

/// A dev/test-friendly backend that checks an environment variable override
/// first, then falls back to a keyed file under `base_dir`.
///
/// This is the backend `cli-framework-oidc`'s token cache uses by default
/// when no other [`SecretStore`] is injected — it preserves the on-disk,
/// zero-config behavior the crate has always had, but reached through the
/// `SecretStore` seam so swapping in e.g. an OpenBao-backed store later
/// requires no caller-side changes.
///
/// - `get`: the environment variable (see
///   [`with_env_prefix`](Self::with_env_prefix)) wins if set, else the file
///   at `<base_dir>/<key segments joined by the OS path separator>` is read.
/// - `put`/`delete` operate on the file only — environment variables are
///   read-only input, there's nowhere to durably write them back to.
/// - Files are written 0600 (unix) via an atomic tmp-then-rename; the
///   containing directories are created 0700.
/// - `rotate` returns [`SecretError::NotSupported`].
pub struct EnvFileSecretStore {
    base_dir: PathBuf,
    env_prefix: Option<String>,
}

impl EnvFileSecretStore {
    /// Store/read keyed files under `base_dir` (created on first write).
    pub fn new(base_dir: impl Into<PathBuf>) -> Self {
        Self {
            base_dir: base_dir.into(),
            env_prefix: None,
        }
    }

    /// Prefix for the environment-variable override lookup, e.g.
    /// `.with_env_prefix("MYAPP")` checks `MYAPP_<KEY>` before falling back
    /// to the file. Without a prefix, the bare uppercased key is checked.
    pub fn with_env_prefix(mut self, prefix: impl Into<String>) -> Self {
        self.env_prefix = Some(prefix.into());
        self
    }

    /// The environment variable name a given key resolves to.
    fn env_var_name(&self, key: &SecretKey) -> String {
        let core: String = key
            .as_str()
            .chars()
            .map(|c| {
                if c.is_ascii_alphanumeric() {
                    c.to_ascii_uppercase()
                } else {
                    '_'
                }
            })
            .collect();
        match &self.env_prefix {
            Some(p) => format!("{p}_{core}"),
            None => core,
        }
    }

    /// The on-disk file path a given key resolves to.
    fn file_path(&self, key: &SecretKey) -> PathBuf {
        let mut p = self.base_dir.clone();
        for seg in key.segments() {
            p.push(seg);
        }
        p
    }
}

#[async_trait]
impl SecretStore for EnvFileSecretStore {
    async fn get(&self, key: &SecretKey) -> Result<SecretValue, SecretError> {
        if let Ok(v) = std::env::var(self.env_var_name(key)) {
            return Ok(SecretValue::from(v));
        }
        let path = self.file_path(key);
        tokio::task::spawn_blocking(move || read_file(&path))
            .await
            .map_err(SecretError::backend)?
    }

    async fn put(&self, key: &SecretKey, value: SecretValue) -> Result<(), SecretError> {
        let path = self.file_path(key);
        let bytes = value.expose().to_vec();
        tokio::task::spawn_blocking(move || write_file(&path, &bytes))
            .await
            .map_err(SecretError::backend)?
    }

    async fn delete(&self, key: &SecretKey) -> Result<(), SecretError> {
        let path = self.file_path(key);
        tokio::task::spawn_blocking(move || delete_file(&path))
            .await
            .map_err(SecretError::backend)?
    }

    async fn rotate(&self, _key: &SecretKey) -> Result<SecretValue, SecretError> {
        Err(SecretError::NotSupported(
            "rotate is not supported by EnvFileSecretStore",
        ))
    }
}

fn read_file(path: &Path) -> Result<SecretValue, SecretError> {
    match std::fs::read(path) {
        Ok(bytes) => Ok(SecretValue::from(bytes)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Err(SecretError::NotFound),
        Err(e) if e.kind() == std::io::ErrorKind::PermissionDenied => {
            Err(SecretError::PermissionDenied)
        }
        Err(e) => Err(SecretError::backend(e)),
    }
}

fn write_file(path: &Path, bytes: &[u8]) -> Result<(), SecretError> {
    let parent = path
        .parent()
        .ok_or_else(|| SecretError::backend("secret path has no parent directory"))?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        std::fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(parent)
            .map_err(SecretError::backend)?;
    }
    #[cfg(not(unix))]
    {
        std::fs::create_dir_all(parent).map_err(SecretError::backend)?;
    }

    let file_name = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("secret");
    let tmp_path = parent.join(format!("{file_name}.tmp.{}", std::process::id()));

    #[cfg(unix)]
    {
        use std::io::Write;
        use std::os::unix::fs::OpenOptionsExt;
        let mut f = std::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o600)
            .open(&tmp_path)
            .map_err(SecretError::backend)?;
        f.write_all(bytes).map_err(SecretError::backend)?;
        f.sync_all().map_err(SecretError::backend)?;
    }
    #[cfg(not(unix))]
    {
        use std::io::Write;
        let mut f = std::fs::File::create(&tmp_path).map_err(SecretError::backend)?;
        f.write_all(bytes).map_err(SecretError::backend)?;
        f.sync_all().map_err(SecretError::backend)?;
    }

    std::fs::rename(&tmp_path, path).map_err(SecretError::backend)?;
    Ok(())
}

fn delete_file(path: &Path) -> Result<(), SecretError> {
    match std::fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(SecretError::backend(e)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn key(s: &str) -> SecretKey {
        SecretKey::parse(s).unwrap()
    }

    #[tokio::test]
    async fn env_override_wins_over_file() {
        let dir = TempDir::new().unwrap();
        let store = EnvFileSecretStore::new(dir.path()).with_env_prefix("CFWTEST");
        let k = key("some/thing");
        std::env::set_var("CFWTEST_SOME_THING", "from-env");
        let v = store.get(&k).await.unwrap();
        assert_eq!(v.expose_str().unwrap(), "from-env");
        std::env::remove_var("CFWTEST_SOME_THING");
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn put_creates_0700_dir_and_0600_file() {
        use std::os::unix::fs::PermissionsExt;
        let dir = TempDir::new().unwrap();
        let base = dir.path().join("nested");
        let store = EnvFileSecretStore::new(&base);
        let k = key("a/b");
        store.put(&k, SecretValue::from("v")).await.unwrap();

        let file_path = base.join("a").join("b");
        assert!(file_path.exists());
        let file_mode = std::fs::metadata(&file_path).unwrap().permissions().mode() & 0o777;
        assert_eq!(file_mode, 0o600);

        let dir_mode = std::fs::metadata(base.join("a"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(dir_mode, 0o700);
    }
}
