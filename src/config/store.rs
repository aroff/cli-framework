//! [`ConfigStore`]: the app-facing handle over a [`super::ConfigBackend`].

use super::{ConfigBackend, ConfigError, ConfigFormat, VersionedConfig};
use std::collections::HashMap;
use std::sync::{Arc, Mutex, RwLock};

const SCHEMA_VERSION_FIELD: &str = "schema_version";

/// A migration step: given the document at `from_version` (as a generic JSON
/// value, regardless of the store's on-disk format), produce the document one
/// version forward. Returning an arbitrary boxed error lets a migration use
/// `?` freely; [`ConfigStore::load`] wraps it as [`ConfigError::MigrationFailed`].
pub type MigrationFn = Arc<
    dyn Fn(serde_json::Value) -> Result<serde_json::Value, Box<dyn std::error::Error + Send + Sync>>
        + Send
        + Sync,
>;

/// Read the stored schema version. A value that does not fit in `u32` is
/// clamped to `u32::MAX` rather than wrapped: `4294967297` truncated with `as
/// u32` would silently read back as `1`, defeating the very check
/// [`ConfigStore::load`] uses this for (a document claiming a version ahead of
/// [`ConfigStore::current_version`] must be refused, never downgraded or
/// migrated as if it were old). Clamping keeps that guard correct — the value
/// is still "too new" either way — without changing the field's public `u32`
/// type in [`ConfigError::VersionTooNew`].
fn read_version(value: &serde_json::Value) -> u32 {
    value
        .get(SCHEMA_VERSION_FIELD)
        .and_then(|v| v.as_u64())
        .map(|v| u32::try_from(v).unwrap_or(u32::MAX))
        .unwrap_or(0)
}

fn write_version(value: &mut serde_json::Value, version: u32) {
    if let serde_json::Value::Object(map) = value {
        map.insert(
            SCHEMA_VERSION_FIELD.to_string(),
            serde_json::Value::from(version),
        );
    }
}

/// Owns the backend, the serialization format, the current schema version,
/// and the ordered migrations for a typed configuration value `T`.
///
/// `load` reads, deserializes, migrates forward, and returns the value
/// *without* writing back — persisting a migration is the caller's decision.
/// `save` stamps the current version, serializes, and writes through the
/// backend, refusing when the backend is read-only. Writers are serialized by
/// an internal lock; readers are not (spec 016 user stories 23-24).
///
/// `ConfigStore` is the single seam this module is tested through — see
/// `tests/unit/config_store.rs` and `tests/unit/config_versioning.rs`.
pub struct ConfigStore<T: VersionedConfig> {
    backend: Arc<dyn ConfigBackend>,
    format: ConfigFormat,
    current_version: u32,
    migrations: HashMap<u32, MigrationFn>,
    current: RwLock<Arc<T>>,
    subscribers: Mutex<Vec<Subscriber<T>>>,
    write_lock: Mutex<()>,
}

type Subscriber<T> = Arc<dyn Fn(&T) + Send + Sync>;

impl<T: VersionedConfig> ConfigStore<T> {
    /// Build a store over `backend`. The cached [`Self::current`] value starts
    /// out as `T::default()` stamped with `current_version` — call
    /// [`Self::resolve`] to actually read the backend.
    pub fn new(
        backend: Arc<dyn ConfigBackend>,
        format: ConfigFormat,
        current_version: u32,
    ) -> Self {
        let mut default = T::default();
        default.set_schema_version(current_version);
        Self {
            backend,
            format,
            current_version,
            migrations: HashMap::new(),
            current: RwLock::new(Arc::new(default)),
            subscribers: Mutex::new(Vec::new()),
            write_lock: Mutex::new(()),
        }
    }

    /// Register a migration that advances the document from `from_version` to
    /// `from_version + 1`. Migrations are run in sequence starting from the
    /// document's stored version up to [`Self::current_version`].
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
        self.migrations.insert(from_version, Arc::new(migration));
        self
    }

    /// The configured serialization format.
    pub fn format(&self) -> ConfigFormat {
        self.format
    }

    /// The schema version this store resolves documents up to.
    pub fn current_version(&self) -> u32 {
        self.current_version
    }

    /// The backend's human-readable label (file path, registry key, ...).
    pub fn backend_label(&self) -> String {
        self.backend.label()
    }

    /// Read the backend, deserialize, and migrate forward to
    /// [`Self::current_version`]. Does **not** update the cached
    /// [`Self::current`] value or write anything back — see [`Self::resolve`]
    /// and [`Self::reload`] for the caching variants.
    ///
    /// An empty backend (nothing stored yet) yields `T::default()` stamped
    /// with the current version. A stored version ahead of
    /// [`Self::current_version`] is refused with
    /// [`ConfigError::VersionTooNew`] rather than silently downgraded. A gap
    /// in the migration chain is refused with [`ConfigError::NoMigrationPath`].
    /// A migration that errors fails the whole load — nothing already
    /// migrated in this call is ever partially applied.
    pub fn load(&self) -> Result<T, ConfigError> {
        let label = self.backend.label();
        let bytes = self.backend.read()?;
        if bytes.is_empty() {
            let mut default = T::default();
            default.set_schema_version(self.current_version);
            return Ok(default);
        }

        let mut value = self.format.bytes_to_value(&label, &bytes)?;
        let mut version = read_version(&value);

        if version > self.current_version {
            return Err(ConfigError::VersionTooNew {
                found: version,
                current: self.current_version,
            });
        }

        while version < self.current_version {
            let migration = self
                .migrations
                .get(&version)
                .ok_or(ConfigError::NoMigrationPath {
                    from_version: version,
                    to_version: self.current_version,
                })?;
            value = migration(value).map_err(|source| ConfigError::MigrationFailed {
                from_version: version,
                source,
            })?;
            version += 1;
            write_version(&mut value, version);
        }

        let mut result: T = serde_json::from_value(value).map_err(|e| ConfigError::Parse {
            backend: label.clone(),
            source: Box::new(e),
        })?;
        result.set_schema_version(self.current_version);
        Ok(result)
    }

    /// Stamp the current schema version onto a clone of `value`, serialize it
    /// through the configured format, and write it through the backend.
    /// Refuses with [`ConfigError::ReadOnly`] (without calling
    /// [`ConfigBackend::write`] at all) when the backend does not support
    /// writes. Concurrent calls are serialized by an internal lock so two
    /// callers saving at once cannot interleave into a broken document.
    pub fn save(&self, value: &T) -> Result<(), ConfigError> {
        if !self.backend.supports_write() {
            return Err(ConfigError::ReadOnly {
                backend: self.backend.label(),
            });
        }

        let _guard = self
            .write_lock
            .lock()
            .expect("ConfigStore write lock poisoned");

        let mut value = value.clone();
        value.set_schema_version(self.current_version);
        let label = self.backend.label();
        let json = serde_json::to_value(&value).map_err(|e| ConfigError::Serialize {
            backend: label.clone(),
            source: Box::new(e),
        })?;
        let bytes = self.format.value_to_bytes(&label, &json)?;
        self.backend.write(&bytes)?;

        *self
            .current
            .write()
            .expect("ConfigStore current-value lock poisoned") = Arc::new(value);
        Ok(())
    }

    /// Run [`Self::load`] once, cache the result, and return it. This is what
    /// `AppBuilder::build()` calls exactly once per app so a one-shot CLI's
    /// resolved value never changes again during that process's lifetime.
    pub fn resolve(&self) -> Result<T, ConfigError> {
        let value = self.load()?;
        *self
            .current
            .write()
            .expect("ConfigStore current-value lock poisoned") = Arc::new(value.clone());
        Ok(value)
    }

    /// The most recently resolved value — from the last [`Self::resolve`],
    /// [`Self::reload`], or [`Self::save`] call. Before any of those have run
    /// it is `T::default()` stamped with the current version.
    pub fn current(&self) -> Arc<T> {
        self.current
            .read()
            .expect("ConfigStore current-value lock poisoned")
            .clone()
    }

    /// Re-run resolution (backend read + migrate) and, on success, replace
    /// the cached value and notify every subscriber registered via
    /// [`Self::subscribe`]. On failure the cached value is left unchanged.
    ///
    /// Subscribers are invoked from a **snapshot taken under the lock, then
    /// invoked after the lock is dropped** — a subscriber is free to call
    /// [`Self::subscribe`] or [`Self::reload`] itself (directly, or
    /// indirectly by triggering something that does) without deadlocking on
    /// `subscribers`, since by the time any callback runs the lock is no
    /// longer held. Invoking callbacks while still holding the lock was
    /// tried and rejected: `std::sync::Mutex` is not reentrant, so a
    /// subscriber calling back into this store from within its own callback
    /// would hang forever, not merely error.
    ///
    /// One-shot CLI applications never call this and keep today's
    /// resolve-once-at-build behavior; long-running applications opt in
    /// (spec 016 user stories 16-18).
    pub fn reload(&self) -> Result<(), ConfigError> {
        let value = self.load()?;
        *self
            .current
            .write()
            .expect("ConfigStore current-value lock poisoned") = Arc::new(value.clone());
        let subs: Vec<Subscriber<T>> = self
            .subscribers
            .lock()
            .expect("ConfigStore subscribers lock poisoned")
            .clone();
        for f in &subs {
            f(&value);
        }
        Ok(())
    }

    /// Register a callback invoked with the new value after every successful
    /// [`Self::reload`].
    pub fn subscribe(&self, f: impl Fn(&T) + Send + Sync + 'static) {
        self.subscribers
            .lock()
            .expect("ConfigStore subscribers lock poisoned")
            .push(Arc::new(f));
    }
}
