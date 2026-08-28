//! [`ConfigError`]: typed errors for the [`super::config`] module.

use std::path::PathBuf;

/// Errors produced by [`super::ConfigBackend`] implementations and
/// [`super::ConfigStore`].
///
/// Deliberately typed (rather than an opaque `anyhow::Error`) so callers can
/// branch on failure mode — e.g. treat [`ConfigError::VersionTooNew`] as "tell
/// the user to upgrade" rather than a generic parse failure. Every variant
/// carries the backend's [`super::ConfigBackend::label`] or a filesystem path
/// so a diagnostic names exactly which store/file/key was involved.
#[derive(thiserror::Error, Debug)]
#[non_exhaustive]
pub enum ConfigError {
    /// A [`super::ConfigBackend::read`] call failed.
    #[error("CE001: failed to read configuration from backend '{backend}': {source}")]
    BackendRead {
        backend: String,
        #[source]
        source: Box<dyn std::error::Error + Send + Sync>,
    },

    /// A [`super::ConfigBackend::write`] call failed.
    #[error("CE002: failed to write configuration to backend '{backend}': {source}")]
    BackendWrite {
        backend: String,
        #[source]
        source: Box<dyn std::error::Error + Send + Sync>,
    },

    /// [`super::ConfigStore::save`] was called against a backend whose
    /// [`super::ConfigBackend::supports_write`] returns `false`. The store
    /// checks this itself and never calls `write` in this case.
    #[error("CE003: backend '{backend}' is read-only; save() was refused")]
    ReadOnly { backend: String },

    /// The stored bytes could not be parsed as the store's configured
    /// [`super::ConfigFormat`], or the parsed document could not be
    /// deserialized into the target type.
    #[error("CE004: failed to parse configuration from backend '{backend}': {source}")]
    Parse {
        backend: String,
        #[source]
        source: Box<dyn std::error::Error + Send + Sync>,
    },

    /// The in-memory value could not be serialized to the store's configured
    /// [`super::ConfigFormat`].
    #[error("CE005: failed to serialize configuration for backend '{backend}': {source}")]
    Serialize {
        backend: String,
        #[source]
        source: Box<dyn std::error::Error + Send + Sync>,
    },

    /// A registered migration returned an error while advancing the document
    /// from `from_version`. `load` fails as a whole — the document already
    /// migrated by prior steps is discarded, never partially applied.
    #[error("CE006: migration from schema version {from_version} failed: {source}")]
    MigrationFailed {
        from_version: u32,
        #[source]
        source: Box<dyn std::error::Error + Send + Sync>,
    },

    /// The document is behind the store's current schema version, but no
    /// migration was registered for `from_version`, so there is no way to
    /// bridge the gap toward `to_version`.
    #[error(
        "CE007: no migration registered to advance schema version {from_version} toward {to_version}"
    )]
    NoMigrationPath { from_version: u32, to_version: u32 },

    /// The stored document's schema version is newer than the version this
    /// binary knows about. Refused rather than silently downgraded — see
    /// spec 016 user story 12.
    #[error(
        "CE008: configuration schema version {found} is newer than this binary supports (current {current}); refusing to downgrade"
    )]
    VersionTooNew { found: u32, current: u32 },

    /// A low-level filesystem failure, distinct from the [`ConfigError::BackendRead`] /
    /// [`ConfigError::BackendWrite`] envelope — used by [`super::FileBackend`] for
    /// path-specific failures (missing parent, permission denied, atomic
    /// rename failure) where a concrete path is more useful than a label.
    #[error("CE009: I/O error at '{}': {source}", path.display())]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
}

impl ConfigError {
    /// Wrap an arbitrary backend read failure as [`ConfigError::BackendRead`].
    pub fn backend_read(
        backend: impl Into<String>,
        source: impl Into<Box<dyn std::error::Error + Send + Sync>>,
    ) -> Self {
        ConfigError::BackendRead {
            backend: backend.into(),
            source: source.into(),
        }
    }

    /// Wrap an arbitrary backend write failure as [`ConfigError::BackendWrite`].
    pub fn backend_write(
        backend: impl Into<String>,
        source: impl Into<Box<dyn std::error::Error + Send + Sync>>,
    ) -> Self {
        ConfigError::BackendWrite {
            backend: backend.into(),
            source: source.into(),
        }
    }
}
