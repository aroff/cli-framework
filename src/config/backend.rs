//! [`ConfigBackend`]: byte-level storage abstraction for [`super::ConfigStore`].

use super::ConfigError;

/// Where a configuration document physically lives.
///
/// A backend deals in **raw bytes only** — it never sees the serialization
/// format ([`super::ConfigFormat`]) or the typed value. This is what lets the
/// same backend serve both JSON and TOML, and lets an application swap where
/// config is stored (file vs. Windows registry) without touching anything
/// else.
///
/// Implementations MUST be safe to hold behind an `Arc` and call concurrently
/// from multiple threads (`Send + Sync`); [`super::ConfigStore`] serializes
/// its own writers with a lock, but readers are not serialized, so `read`
/// must be safe to call concurrently with itself and with `write`.
pub trait ConfigBackend: Send + Sync {
    /// Read the raw bytes currently stored.
    ///
    /// An absent or empty backend (nothing saved yet) returns `Ok(Vec::new())`
    /// — this is not an error at this layer. [`super::ConfigStore::load`] maps
    /// an empty read to the type's `Default`, which is what lets a first run
    /// work without shipping a template file (spec 016 user story 5).
    fn read(&self) -> Result<Vec<u8>, ConfigError>;

    /// Atomically overwrite the stored bytes.
    ///
    /// Implementations that write to a filesystem MUST make this atomic (a
    /// crash or power loss must never leave a truncated document) and MUST
    /// create any missing parent directories on first write. See
    /// [`super::FileBackend`] for the reference implementation.
    fn write(&self, bytes: &[u8]) -> Result<(), ConfigError>;

    /// Whether [`Self::write`] is expected to succeed.
    ///
    /// [`super::ConfigStore::save`] checks this *before* calling `write` and
    /// returns [`ConfigError::ReadOnly`] without ever invoking it when this
    /// returns `false`.
    fn supports_write(&self) -> bool;

    /// A human-readable identifier for diagnostics — e.g. the file path or
    /// registry key. Surfaced in every [`ConfigError`] variant and by
    /// `doctor` so a support engineer can tell exactly which file or
    /// registry key an app is reading (spec 016 user story 22).
    fn label(&self) -> String;
}
