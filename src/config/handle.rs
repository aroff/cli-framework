//! [`ConfigHandle`]: the object-safe accessor reachable via `AppContext::opt_config_handle`.

use super::{ConfigError, ConfigStore, VersionedConfig};

/// A non-generic, object-safe view over a [`ConfigStore<T>`], for the one
/// case `T` cannot be named: `AppContext::opt_config_handle`.
///
/// `AppContext` is implemented by each application's own concrete type, and
/// its framework-owned accessors (`opt_registry`, `telemetry()`) work as
/// defaulted trait methods because their return types are fixed, non-generic
/// types the trait can name. `ConfigStore<T>` cannot be named that way — `T`
/// is a different type in every application, and a generic method is not
/// object-safe on a trait used polymorphically. `ConfigHandle` exposes only
/// the type-erased operations the framework itself needs: `reload()`, and a
/// raw JSON read/write for generic tooling (a future `config` command group,
/// `doctor`) that has no reason to know `T`. The **typed** resolved value is
/// never threaded through `AppContext` — see `AppBuilder::build_with_config`
/// and `App::config_store` for how an application gets that back instead.
///
/// See spec 016, "Access, and why it cannot mirror the `Telemetry` handle
/// exactly."
pub trait ConfigHandle: Send + Sync {
    /// Re-run resolution and, on success, replace the cached value and
    /// notify subscribers registered on the typed [`ConfigStore`].
    fn reload(&self) -> Result<(), ConfigError>;

    /// The backend's human-readable label (file path, registry key, ...).
    fn backend_label(&self) -> String;

    /// The current resolved value as JSON, regardless of the store's
    /// configured on-disk format. This is `T` serialized to JSON, not the raw
    /// on-disk bytes — a field `T` doesn't declare will not appear here even
    /// if it were somehow present in the backend.
    fn current_json(&self) -> Result<serde_json::Value, ConfigError>;

    /// Set the stored document from a JSON value, for callers with no reason
    /// to name `T` (a generic `config` command, `doctor`). This does **not**
    /// bypass `T`: the value is deserialized into `T` and re-serialized
    /// through [`ConfigStore::save`] exactly as a typed caller's value would
    /// be, so it is validated against `T`'s schema (JSON that doesn't parse
    /// as `T` is rejected, matching [`ConfigError::Parse`]) and normalized to
    /// it (a field `T` doesn't declare is silently dropped, since there is
    /// nowhere in `T` for it to round-trip through). Fails the same way
    /// [`ConfigStore::save`] does against a read-only backend.
    fn save_json(&self, value: serde_json::Value) -> Result<(), ConfigError>;
}

// `ConfigStore<T>` has inherent methods with the same names as several of
// these (`reload`, `backend_label`). Calls below use fully-qualified
// `ConfigStore::method(self)` syntax to reach those inherent implementations
// unambiguously, rather than relying on inherent-over-trait shadowing rules.
impl<T> ConfigHandle for ConfigStore<T>
where
    T: VersionedConfig,
{
    fn reload(&self) -> Result<(), ConfigError> {
        ConfigStore::reload(self)
    }

    fn backend_label(&self) -> String {
        ConfigStore::backend_label(self)
    }

    fn current_json(&self) -> Result<serde_json::Value, ConfigError> {
        let current = ConfigStore::current(self);
        serde_json::to_value(&*current).map_err(|e| ConfigError::Serialize {
            backend: ConfigStore::backend_label(self),
            source: Box::new(e),
        })
    }

    fn save_json(&self, value: serde_json::Value) -> Result<(), ConfigError> {
        let typed: T = serde_json::from_value(value).map_err(|e| ConfigError::Parse {
            backend: ConfigStore::backend_label(self),
            source: Box::new(e),
        })?;
        ConfigStore::save(self, &typed)
    }
}
