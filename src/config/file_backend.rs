//! [`FileBackend`]: a [`ConfigBackend`] backed by a file in the user profile.

use super::{ConfigBackend, ConfigError};
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

/// Process-wide counter mixed into every temp file name (alongside the PID),
/// so two `FileBackend`/`ConfigStore` instances in the *same* process that
/// happen to target the same path cannot collide on the same temp file. PID
/// alone distinguishes across processes but not within one: `ConfigStore`'s
/// write lock only serializes writers sharing a single store instance, and
/// nothing prevents an application (or its tests) from constructing two
/// instances over the same underlying path.
static WRITE_COUNTER: AtomicU64 = AtomicU64::new(0);

/// A [`ConfigBackend`] storing the document as a single file.
///
/// Writes are atomic: [`Self::write`] creates a temporary file in the same
/// directory as the target, writes and `fsync`s it, then renames it over the
/// target. A crash or power loss at any point before the rename leaves the
/// previous file exactly as it was; the temporary file is only ever visible
/// mid-write, never left behind on either success or (most) failure paths.
/// Missing parent directories are created automatically on first write.
pub struct FileBackend {
    path: PathBuf,
}

impl FileBackend {
    /// Store the document at an explicit `path`.
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    /// Store the document under the platform config directory
    /// (`dirs::config_dir()`) at `<config_dir>/<app_name>/config`.
    ///
    /// The file carries no extension: the backend never knows which
    /// [`super::ConfigFormat`] the store above it is using, so a fixed,
    /// format-neutral filename avoids implying JSON or TOML content from the
    /// name alone.
    ///
    /// Returns [`ConfigError::Io`] if the platform has no resolvable config
    /// directory (no `$HOME`/`$XDG_CONFIG_HOME` equivalent).
    pub fn for_app(app_name: &str) -> Result<Self, ConfigError> {
        let base = dirs::config_dir().ok_or_else(|| ConfigError::Io {
            path: PathBuf::from(app_name),
            source: io::Error::new(
                io::ErrorKind::NotFound,
                "no platform config directory available",
            ),
        })?;
        Ok(Self::new(base.join(app_name).join("config")))
    }

    /// The path this backend reads and writes.
    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl ConfigBackend for FileBackend {
    fn read(&self) -> Result<Vec<u8>, ConfigError> {
        match std::fs::read(&self.path) {
            Ok(bytes) => Ok(bytes),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(Vec::new()),
            Err(e) => Err(ConfigError::Io {
                path: self.path.clone(),
                source: e,
            }),
        }
    }

    fn write(&self, bytes: &[u8]) -> Result<(), ConfigError> {
        let parent = self.path.parent().ok_or_else(|| ConfigError::Io {
            path: self.path.clone(),
            source: io::Error::new(io::ErrorKind::InvalidInput, "config path has no parent"),
        })?;

        std::fs::create_dir_all(parent).map_err(|e| ConfigError::Io {
            path: parent.to_path_buf(),
            source: e,
        })?;

        let file_name = self
            .path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("config");
        let attempt = WRITE_COUNTER.fetch_add(1, Ordering::Relaxed);
        let tmp_path = parent.join(format!("{file_name}.tmp.{}.{attempt}", std::process::id()));

        if let Err(err) = write_and_rename(&tmp_path, &self.path, bytes) {
            // Best-effort cleanup: a half-written temp file must never
            // survive a failed write, even though the *target* file (if any)
            // is untouched because the rename never happened.
            let _ = std::fs::remove_file(&tmp_path);
            return Err(err);
        }

        Ok(())
    }

    fn supports_write(&self) -> bool {
        true
    }

    fn label(&self) -> String {
        format!("file:{}", self.path.display())
    }
}

/// Write `bytes` to `tmp_path` and rename it over `target`, mapping every
/// failure to [`ConfigError::Io`] with whichever path was involved. Split out
/// of [`FileBackend::write`] so the caller can clean up `tmp_path` uniformly
/// on any error branch.
fn write_and_rename(tmp_path: &Path, target: &Path, bytes: &[u8]) -> Result<(), ConfigError> {
    use std::io::Write;

    let mut f = std::fs::File::create(tmp_path).map_err(|e| ConfigError::Io {
        path: tmp_path.to_path_buf(),
        source: e,
    })?;
    f.write_all(bytes).map_err(|e| ConfigError::Io {
        path: tmp_path.to_path_buf(),
        source: e,
    })?;
    f.sync_all().map_err(|e| ConfigError::Io {
        path: tmp_path.to_path_buf(),
        source: e,
    })?;
    drop(f);

    std::fs::rename(tmp_path, target).map_err(|e| ConfigError::Io {
        path: target.to_path_buf(),
        source: e,
    })
}

// `write_and_rename` is module-private, reachable only from within this file
// (the house exception for tiny pure-function-shaped helpers). Its
// `write_all`/`sync_all` error arms are not reachable through the public
// `write` API in a portable, non-flaky way — `write` always constructs its
// own `tmp_path`, and a *directory* permission change (used elsewhere for
// the `File::create` arm) can't selectively fail a write to an already-open
// file descriptor. Calling this function directly lets the test supply
// `/dev/full` as `tmp_path`: a real Linux device that accepts opens but
// fails every `write(2)` with `ENOSPC`, deterministically and without
// touching any process-wide state (unlike, say, an `RLIMIT_FSIZE` change,
// which would affect every other test running concurrently in this binary).
#[cfg(all(test, target_os = "linux"))]
mod tests {
    use super::*;

    #[test]
    fn write_all_failure_on_dev_full_returns_io_error() {
        // `/dev/full` is a standard kernel-provided character device present
        // on every mainstream Linux distribution's `/dev` (devtmpfs); if a
        // future CI image genuinely lacks it, failing loudly here is more
        // useful than a silent skip that quietly stops proving anything.
        let err = write_and_rename(Path::new("/dev/full"), Path::new("/dev/null"), b"x")
            .expect_err("/dev/full must refuse the write");
        assert!(matches!(err, ConfigError::Io { .. }));
    }
}
