//! Schema-versioning and migration-pipeline behavior of `ConfigStore`.
//!
//! These tests operate directly on the JSON bytes an older release would
//! have written (rather than constructing them via a same-shaped struct), so
//! they exercise the exact "a document written by an older/newer release"
//! scenarios spec 016 describes, independent of whatever the *current*
//! struct shape happens to be.

use cli_framework::config::{
    ConfigError, ConfigFormat, ConfigStore, InMemoryBackend, VersionedConfig,
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

#[derive(Default, Clone, Debug, PartialEq, Serialize, Deserialize)]
struct AppConfig {
    schema_version: u32,
    // v1 had `name`; v2 renamed it to `full_name`. v3 added `retries`.
    full_name: String,
    // `#[serde(default)]` because a v2 document (before `retries` existed)
    // must still deserialize into this struct when a test only exercises
    // the v1->v2 rename in isolation, without also running the v2->v3
    // migration that would populate it explicitly.
    #[serde(default)]
    retries: u32,
}

impl VersionedConfig for AppConfig {
    fn schema_version(&self) -> u32 {
        self.schema_version
    }
    fn set_schema_version(&mut self, version: u32) {
        self.schema_version = version;
    }
}

fn v1_bytes(name: &str) -> Vec<u8> {
    serde_json::to_vec(&serde_json::json!({
        "schema_version": 1,
        "name": name,
    }))
    .unwrap()
}

fn v2_bytes(full_name: &str) -> Vec<u8> {
    serde_json::to_vec(&serde_json::json!({
        "schema_version": 2,
        "full_name": full_name,
    }))
    .unwrap()
}

fn rename_name_to_full_name(
    mut value: serde_json::Value,
) -> Result<serde_json::Value, Box<dyn std::error::Error + Send + Sync>> {
    if let serde_json::Value::Object(map) = &mut value {
        if let Some(name) = map.remove("name") {
            map.insert("full_name".to_string(), name);
        }
    }
    Ok(value)
}

fn add_default_retries(
    mut value: serde_json::Value,
) -> Result<serde_json::Value, Box<dyn std::error::Error + Send + Sync>> {
    if let serde_json::Value::Object(map) = &mut value {
        map.insert("retries".to_string(), serde_json::Value::from(3));
    }
    Ok(value)
}

// User stories 9-10 — a document at version 1 loaded by a binary at version 3
// with registered 1->2 and 2->3 migrations arrives fully migrated, in
// sequence (not skipping straight to the last migration).
#[test]
fn migrations_1_to_2_to_3_apply_in_sequence() {
    let backend = Arc::new(InMemoryBackend::with_bytes(v1_bytes("Ada")));
    let store = ConfigStore::<AppConfig>::new(backend, ConfigFormat::default(), 3)
        .with_migration(1, rename_name_to_full_name)
        .with_migration(2, add_default_retries);

    let cfg = store.load().unwrap();
    assert_eq!(cfg.full_name, "Ada");
    assert_eq!(cfg.retries, 3);
    assert_eq!(cfg.schema_version, 3);
}

// A document already at the current version skips migrations entirely.
#[test]
fn document_at_current_version_skips_migrations() {
    let backend = Arc::new(InMemoryBackend::with_bytes(v2_bytes("Grace")));
    let store = ConfigStore::<AppConfig>::new(backend, ConfigFormat::default(), 2)
        .with_migration(1, rename_name_to_full_name);
    let cfg = store.load().unwrap();
    assert_eq!(cfg.full_name, "Grace");
    assert_eq!(cfg.schema_version, 2);
}

// User story 12 — a document at version 3 loaded by a binary at version 2 is
// refused with the typed error, never downgraded.
#[test]
fn version_ahead_of_binary_is_refused_not_downgraded() {
    let backend = Arc::new(InMemoryBackend::with_bytes(
        serde_json::to_vec(
            &serde_json::json!({"schema_version": 3, "full_name": "x", "retries": 1}),
        )
        .unwrap(),
    ));
    let store = ConfigStore::<AppConfig>::new(backend.clone(), ConfigFormat::default(), 2);
    let err = store.load().unwrap_err();
    assert!(matches!(
        err,
        ConfigError::VersionTooNew {
            found: 3,
            current: 2
        }
    ));
    assert!(err.to_string().contains("CE008"));

    // Negative check: the same bytes at the binary's own version load fine,
    // proving the refusal above is really about the version gap, not the
    // document shape.
    backend.set_bytes(v2_bytes("y"));
    assert!(store.load().is_ok());
}

// Regression: a stored schema_version too large to fit in u32 must be
// refused, not silently truncated into a small, plausible-looking version
// that then gets migrated as if it were genuinely old. `4294967297` (2^32 +
// 1) truncated with a bare `as u32` reads back as `1` — verified this test
// fails against that exact truncation before the fix landed.
#[test]
fn schema_version_overflowing_u32_is_refused_not_wrapped() {
    let backend = Arc::new(InMemoryBackend::with_bytes(
        serde_json::to_vec(&serde_json::json!({
            "schema_version": 4294967297_u64,
            "full_name": "x",
            "retries": 1
        }))
        .unwrap(),
    ));
    let store = ConfigStore::<AppConfig>::new(backend, ConfigFormat::default(), 2);
    let err = store.load().unwrap_err();
    assert!(
        matches!(err, ConfigError::VersionTooNew { current: 2, .. }),
        "expected VersionTooNew, got {err:?}"
    );
}

// User story 11 — a migration that returns an error causes load to fail
// without partially applying; a subsequent successful load (after fixing the
// underlying data) proves the store's own state was never corrupted by the
// failed attempt.
#[test]
fn failing_migration_does_not_partially_apply() {
    fn always_fails(
        _value: serde_json::Value,
    ) -> Result<serde_json::Value, Box<dyn std::error::Error + Send + Sync>> {
        Err("migration deliberately failed".into())
    }

    let backend = Arc::new(InMemoryBackend::with_bytes(v1_bytes("Ada")));
    let store = ConfigStore::<AppConfig>::new(backend.clone(), ConfigFormat::default(), 2)
        .with_migration(1, always_fails);

    let err = store.load().unwrap_err();
    assert!(matches!(
        err,
        ConfigError::MigrationFailed {
            from_version: 1,
            ..
        }
    ));
    assert!(err.to_string().contains("CE006"));

    // The backend bytes are untouched (load never writes back) and a store
    // with the real migration registered instead succeeds against the same
    // stored bytes — proving nothing was consumed/corrupted by the failure.
    let recovering_store = ConfigStore::<AppConfig>::new(backend, ConfigFormat::default(), 2)
        .with_migration(1, rename_name_to_full_name);
    assert_eq!(recovering_store.load().unwrap().full_name, "Ada");
}

// A gap in the migration chain (no migration registered for the version the
// document is actually at) is refused rather than silently skipped.
#[test]
fn missing_migration_in_chain_is_refused() {
    let backend = Arc::new(InMemoryBackend::with_bytes(v1_bytes("Ada")));
    // Binary is at version 3 but only registers the 2->3 step, not 1->2.
    let store = ConfigStore::<AppConfig>::new(backend, ConfigFormat::default(), 3)
        .with_migration(2, add_default_retries);

    let err = store.load().unwrap_err();
    assert!(matches!(
        err,
        ConfigError::NoMigrationPath {
            from_version: 1,
            to_version: 3
        }
    ));
    assert!(err.to_string().contains("CE007"));
}

// A migration function can run to completion successfully (no
// `MigrationFailed`) yet still leave a document shape the target struct
// cannot deserialize (e.g. it forgot to populate a required field with the
// right type). That is a distinct failure mode from a migration returning
// `Err` — it surfaces as `ConfigError::Parse` on the final deserialize step.
#[test]
fn migration_producing_wrong_shape_surfaces_as_parse_error() {
    fn corrupt_full_name_type(
        mut value: serde_json::Value,
    ) -> Result<serde_json::Value, Box<dyn std::error::Error + Send + Sync>> {
        if let serde_json::Value::Object(map) = &mut value {
            // `full_name` must be a String; a migration bug leaves a number.
            map.insert("full_name".to_string(), serde_json::Value::from(42));
        }
        Ok(value)
    }

    let backend = Arc::new(InMemoryBackend::with_bytes(v1_bytes("Ada")));
    let store = ConfigStore::<AppConfig>::new(backend, ConfigFormat::default(), 2)
        .with_migration(1, corrupt_full_name_type);

    let err = store.load().unwrap_err();
    assert!(matches!(err, ConfigError::Parse { .. }));
    assert!(err.to_string().contains("CE004"));
}

// Malformed stored bytes surface as a typed parse error, not a panic.
#[test]
fn malformed_bytes_surface_as_parse_error() {
    let backend = Arc::new(InMemoryBackend::with_bytes(b"{not valid json".to_vec()));
    let store = ConfigStore::<AppConfig>::new(backend, ConfigFormat::default(), 1);
    let err = store.load().unwrap_err();
    assert!(matches!(err, ConfigError::Parse { .. }));
    assert!(err.to_string().contains("CE004"));
}

// A document missing the schema_version field entirely is treated as version
// 0 (pre-versioning), not an error — so a migration registered for 0 can
// still bring it forward.
#[test]
fn missing_version_field_treated_as_version_zero() {
    let backend = Arc::new(InMemoryBackend::with_bytes(
        serde_json::to_vec(&serde_json::json!({"full_name": "no-version"})).unwrap(),
    ));
    let store = ConfigStore::<AppConfig>::new(backend, ConfigFormat::default(), 1).with_migration(
        0,
        |mut value| {
            if let serde_json::Value::Object(map) = &mut value {
                map.entry("retries").or_insert(serde_json::Value::from(0));
            }
            Ok(value)
        },
    );
    let cfg = store.load().unwrap();
    assert_eq!(cfg.full_name, "no-version");
    assert_eq!(cfg.schema_version, 1);
}
