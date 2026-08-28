//! [`ConfigOptions`]: the format/version/migrations bundle passed to `AppBuilder::with_config`.

use super::store::MigrationFn;
use super::{ConfigFormat, VersionedConfig};
use std::marker::PhantomData;
use std::sync::Arc;

/// Format, current schema version, and migrations for a typed configuration
/// `T`, passed to `AppBuilder::with_config::<T>(options)`.
///
/// Mirrors `TelemetryConfig`'s shape as the value handed to a `with_*`
/// builder method: a plain, independently-constructible struct rather than a
/// nested sub-builder, so it composes with `AppBuilder`'s existing fluent
/// method-chaining style.
pub struct ConfigOptions<T: VersionedConfig> {
    pub(crate) format: ConfigFormat,
    pub(crate) current_version: u32,
    pub(crate) migrations: Vec<(u32, MigrationFn)>,
    _marker: PhantomData<fn() -> T>,
}

impl<T: VersionedConfig> ConfigOptions<T> {
    /// Start building options for a store whose current schema version is
    /// `current_version`. Format defaults to [`ConfigFormat::Json`].
    pub fn new(current_version: u32) -> Self {
        Self {
            format: ConfigFormat::default(),
            current_version,
            migrations: Vec::new(),
            _marker: PhantomData,
        }
    }

    /// Select the on-disk serialization format. Default: JSON.
    pub fn with_format(mut self, format: ConfigFormat) -> Self {
        self.format = format;
        self
    }

    /// Register a migration advancing the document from `from_version` to
    /// `from_version + 1`. Migrations registered here are applied to the
    /// `ConfigStore<T>` built internally by `AppBuilder::build`.
    pub fn with_migration(
        mut self,
        from_version: u32,
        migration: impl Fn(
                serde_json::Value,
            ) -> Result<serde_json::Value, Box<dyn std::error::Error + Send + Sync>>
            + Send
            + Sync
            + 'static,
    ) -> Self {
        self.migrations.push((from_version, Arc::new(migration)));
        self
    }
}

impl<T: VersionedConfig> Default for ConfigOptions<T> {
    /// Current schema version `1` with no migrations — the common case for a
    /// brand-new configuration type that has never shipped a prior schema.
    fn default() -> Self {
        Self::new(1)
    }
}
