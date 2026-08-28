//! [`InMemoryBackend`]: a process-local [`ConfigBackend`] for tests.

use super::{ConfigBackend, ConfigError};
use std::sync::RwLock;

/// A [`ConfigBackend`] backed by a process-local buffer.
///
/// This is the seam spec 016 tests [`super::ConfigStore`] through: versioning,
/// migration, format selection, and error mapping are all store behavior
/// independent of any real filesystem, so they're exercised here rather than
/// against [`super::FileBackend`]. Also supports a read-only mode, used to
/// test that `ConfigStore::save` refuses a non-writable backend without a
/// real filesystem permission dance.
pub struct InMemoryBackend {
    bytes: RwLock<Vec<u8>>,
    writable: bool,
    label: String,
}

impl InMemoryBackend {
    /// An empty, writable backend — reads as defaults until something is
    /// saved.
    pub fn new() -> Self {
        Self {
            bytes: RwLock::new(Vec::new()),
            writable: true,
            label: "in-memory".to_string(),
        }
    }

    /// A writable backend pre-populated with `bytes`, as if written by a
    /// prior run.
    pub fn with_bytes(bytes: impl Into<Vec<u8>>) -> Self {
        Self {
            bytes: RwLock::new(bytes.into()),
            writable: true,
            label: "in-memory".to_string(),
        }
    }

    /// Make this backend refuse writes with [`ConfigError::ReadOnly`] — used
    /// to test that `ConfigStore::save` respects `supports_write`.
    pub fn read_only(mut self) -> Self {
        self.writable = false;
        self
    }

    /// Override the diagnostic label (default: `"in-memory"`).
    pub fn with_label(mut self, label: impl Into<String>) -> Self {
        self.label = label.into();
        self
    }

    /// A snapshot of the currently stored bytes.
    pub fn snapshot(&self) -> Vec<u8> {
        self.bytes
            .read()
            .expect("InMemoryBackend lock poisoned")
            .clone()
    }

    /// Overwrite the stored bytes directly, bypassing `write`/the
    /// read-only flag — simulates an out-of-band edit (e.g. a settings file
    /// hand-edited or written by another process) for `reload()` tests.
    pub fn set_bytes(&self, bytes: impl Into<Vec<u8>>) {
        *self.bytes.write().expect("InMemoryBackend lock poisoned") = bytes.into();
    }
}

impl Default for InMemoryBackend {
    fn default() -> Self {
        Self::new()
    }
}

impl ConfigBackend for InMemoryBackend {
    fn read(&self) -> Result<Vec<u8>, ConfigError> {
        Ok(self
            .bytes
            .read()
            .expect("InMemoryBackend lock poisoned")
            .clone())
    }

    fn write(&self, bytes: &[u8]) -> Result<(), ConfigError> {
        if !self.writable {
            return Err(ConfigError::ReadOnly {
                backend: self.label.clone(),
            });
        }
        *self.bytes.write().expect("InMemoryBackend lock poisoned") = bytes.to_vec();
        Ok(())
    }

    fn supports_write(&self) -> bool {
        self.writable
    }

    fn label(&self) -> String {
        self.label.clone()
    }
}
