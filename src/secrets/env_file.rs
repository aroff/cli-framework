//! [`EnvFileSecretStore`]: the zero-config dev/default backend.

use super::{SecretError, SecretKey, SecretStore, SecretValue};
use async_trait::async_trait;
use std::io::Read;
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
/// - Files are created with their private policy already attached and replaced
///   atomically: mode 0600 under newly-created 0700 directories on Unix; a
///   protected current-user/SYSTEM/Administrators DACL on Windows. Reads
///   validate the opened handle and fail closed for insecure legacy files.
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
    match crate::security::private_file::open_private(path) {
        Ok(mut file) => {
            let mut bytes = Vec::new();
            file.read_to_end(&mut bytes).map_err(SecretError::backend)?;
            Ok(SecretValue::from(bytes))
        }
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

    crate::security::private_file::replace_private(path, bytes).map_err(SecretError::backend)?;
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
        let store =
            EnvFileSecretStore::new(dir.path()).with_env_prefix("CFW_TEST_ENV_FILE_OVERRIDE");
        let k = key("some/thing");
        std::env::set_var("CFW_TEST_ENV_FILE_OVERRIDE_SOME_THING", "from-env");
        let v = store.get(&k).await.unwrap();
        assert_eq!(v.expose_str().unwrap(), "from-env");
        std::env::remove_var("CFW_TEST_ENV_FILE_OVERRIDE_SOME_THING");
    }

    #[tokio::test]
    async fn file_lifecycle_and_fail_closed_reads() {
        let dir = TempDir::new().unwrap();
        let store = EnvFileSecretStore::new(dir.path());
        let k = key("lifecycle/value");

        assert!(matches!(store.get(&k).await, Err(SecretError::NotFound)));
        store.put(&k, SecretValue::from("first")).await.unwrap();
        assert_eq!(store.get(&k).await.unwrap().expose_str().unwrap(), "first");
        store.put(&k, SecretValue::from("second")).await.unwrap();
        assert_eq!(store.get(&k).await.unwrap().expose_str().unwrap(), "second");
        assert!(matches!(
            store.rotate(&k).await,
            Err(SecretError::NotSupported(_))
        ));

        store.delete(&k).await.unwrap();
        store.delete(&k).await.unwrap();
        assert!(matches!(store.get(&k).await, Err(SecretError::NotFound)));
    }

    #[cfg(any(unix, windows))]
    #[tokio::test]
    async fn insecure_existing_file_is_rejected_then_repaired_by_put() {
        #[cfg(unix)]
        use std::os::unix::fs::PermissionsExt;

        let dir = TempDir::new().unwrap();
        let store = EnvFileSecretStore::new(dir.path());
        let k = key("legacy");
        let path = dir.path().join("legacy");
        std::fs::write(&path, b"exposed").unwrap();
        #[cfg(unix)]
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
        assert!(matches!(
            store.get(&k).await,
            Err(SecretError::PermissionDenied)
        ));

        store.put(&k, SecretValue::from("safe")).await.unwrap();
        assert_eq!(store.get(&k).await.unwrap().expose_str().unwrap(), "safe");
        #[cfg(unix)]
        assert_eq!(
            std::fs::metadata(path).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }

    #[test]
    fn backend_error_branches_preserve_non_policy_io_failures() {
        let dir = TempDir::new().unwrap();
        assert!(matches!(
            read_file(Path::new("invalid\0path")),
            Err(SecretError::Backend { .. })
        ));

        let nonempty = dir.path().join("nonempty");
        std::fs::create_dir(&nonempty).unwrap();
        std::fs::write(nonempty.join("child"), b"x").unwrap();
        assert!(matches!(
            delete_file(&nonempty),
            Err(SecretError::Backend { .. })
        ));
        assert!(matches!(
            write_file(Path::new(""), b"x"),
            Err(SecretError::Backend { .. })
        ));
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
