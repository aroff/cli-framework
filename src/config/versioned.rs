//! [`VersionedConfig`]: the schema-version contract [`super::ConfigStore`] requires.

/// A configuration type that carries its own schema version.
///
/// [`super::ConfigStore`] stamps the version on every [`super::ConfigStore::save`]
/// and reads it back on [`super::ConfigStore::load`] to decide whether
/// migrations need to run. The version is expected to serialize to a JSON
/// field literally named `schema_version` (see the `#[derive(Serialize)]`
/// output for the implementing type) — the store reads/writes that field on
/// the generic [`serde_json::Value`] representation while running migrations,
/// independent of `T`'s own field name for it internally.
///
/// The supertrait bounds are implementation necessities every config type
/// naturally has anyway: serializable (to persist it), `Default` (so an
/// empty/absent backend yields sensible defaults — spec 016 user story 5),
/// `Clone` (the store hands out `Arc<T>` snapshots and caches values
/// independent of what a caller does with them afterward).
pub trait VersionedConfig:
    Default + Clone + serde::Serialize + serde::de::DeserializeOwned + Send + Sync + 'static
{
    /// The schema version this value was loaded (or last saved) as.
    fn schema_version(&self) -> u32;

    /// Stamp `version` onto this value. Called by the store on every `save`
    /// and after a successful `load`/migration, so `T`'s own field always
    /// agrees with what was actually persisted.
    fn set_schema_version(&mut self, version: u32);
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::{Deserialize, Serialize};

    #[derive(Default, Clone, Serialize, Deserialize)]
    struct Demo {
        schema_version: u32,
        name: String,
    }

    impl VersionedConfig for Demo {
        fn schema_version(&self) -> u32 {
            self.schema_version
        }
        fn set_schema_version(&mut self, version: u32) {
            self.schema_version = version;
        }
    }

    /// A minimal pure-function sanity check that a type implementing the
    /// trait actually round-trips get/set — the bulk of behavior is tested
    /// through `ConfigStore` in `tests/unit/config_store.rs`.
    #[test]
    fn get_set_round_trip() {
        let mut d = Demo::default();
        assert_eq!(d.schema_version(), 0);
        d.set_schema_version(3);
        assert_eq!(d.schema_version(), 3);
    }
}
