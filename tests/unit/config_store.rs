//! `ConfigStore` behavior tested against `InMemoryBackend` — the seam spec
//! 016 designates for testing versioning, migration, format, and error
//! mapping without touching a filesystem. Filesystem-specific behavior
//! (atomic writes, missing parent dirs) lives in `config_backend_file.rs`;
//! the migration pipeline specifically lives in `config_versioning.rs`.

use cli_framework::config::{
    ConfigBackend, ConfigError, ConfigFormat, ConfigHandle, ConfigStore, InMemoryBackend,
    VersionedConfig,
};
use serde::{Deserialize, Serialize};
use std::sync::{Arc, Mutex};

#[derive(Default, Clone, Debug, PartialEq, Serialize, Deserialize)]
struct DemoConfig {
    schema_version: u32,
    greeting: String,
    count: u32,
}

impl VersionedConfig for DemoConfig {
    fn schema_version(&self) -> u32 {
        self.schema_version
    }
    fn set_schema_version(&mut self, version: u32) {
        self.schema_version = version;
    }
}

fn store_over(backend: InMemoryBackend) -> ConfigStore<DemoConfig> {
    ConfigStore::new(Arc::new(backend), ConfigFormat::default(), 1)
}

// User story 5 — empty/absent backend yields defaults, not a parse error.
#[test]
fn load_on_empty_backend_returns_defaults() {
    let store = store_over(InMemoryBackend::new());
    let cfg = store.load().unwrap();
    assert_eq!(cfg.greeting, DemoConfig::default().greeting);
    assert_eq!(cfg.count, DemoConfig::default().count);
    assert_eq!(
        cfg.schema_version, 1,
        "default stamped with current version"
    );
}

// User stories 2-3 (round trip half) — save then load returns saved values.
#[test]
fn save_then_load_round_trips() {
    let store = store_over(InMemoryBackend::new());
    let mut cfg = store.load().unwrap();
    cfg.greeting = "hello".to_string();
    cfg.count = 42;
    store.save(&cfg).unwrap();

    let reloaded = store.load().unwrap();
    assert_eq!(reloaded.greeting, "hello");
    assert_eq!(reloaded.count, 42);
    assert_eq!(reloaded.schema_version, 1);
}

// `save` always stamps the store's current version onto the value, even if
// the caller's in-memory copy carried a stale one.
#[test]
fn save_stamps_current_version_regardless_of_caller_value() {
    let store = store_over(InMemoryBackend::new());
    let mut cfg = DemoConfig::default();
    cfg.schema_version = 999; // caller-side garbage; store must overwrite it
    store.save(&cfg).unwrap();
    assert_eq!(store.load().unwrap().schema_version, 1);
}

// User story 13/14 — JSON and TOML round-trip the same value.
#[test]
fn json_and_toml_round_trip_same_value() {
    for format in [ConfigFormat::Json, ConfigFormat::Toml] {
        let store = ConfigStore::<DemoConfig>::new(Arc::new(InMemoryBackend::new()), format, 1);
        let mut cfg = store.load().unwrap();
        cfg.greeting = "world".to_string();
        cfg.count = 7;
        store.save(&cfg).unwrap();
        let reloaded = store.load().unwrap();
        assert_eq!(reloaded.greeting, "world");
        assert_eq!(reloaded.count, 7);
    }
}

// Switching format requires no other change — same API, same assertions,
// only the `ConfigFormat` argument differs (covered structurally by the loop
// above; this test additionally checks the two formats actually produce
// different bytes on disk, proving the format selection is not a no-op).
#[test]
fn json_and_toml_produce_different_bytes() {
    let json_backend = Arc::new(InMemoryBackend::new());
    let json_store = ConfigStore::<DemoConfig>::new(json_backend.clone(), ConfigFormat::Json, 1);
    let toml_backend = Arc::new(InMemoryBackend::new());
    let toml_store = ConfigStore::<DemoConfig>::new(toml_backend.clone(), ConfigFormat::Toml, 1);

    let mut cfg = DemoConfig::default();
    cfg.greeting = "fmt-check".to_string();
    json_store.save(&cfg).unwrap();
    toml_store.save(&cfg).unwrap();

    let json_bytes = json_backend.snapshot();
    let toml_bytes = toml_backend.snapshot();
    assert_ne!(json_bytes, toml_bytes);
    assert!(String::from_utf8(json_bytes).unwrap().contains('{'));
    assert!(!String::from_utf8(toml_bytes).unwrap().contains('{'));
}

// TOML-specific parse failures: invalid UTF-8 and syntactically invalid TOML
// both surface as `ConfigError::Parse`, exercising both error-mapping arms in
// `ConfigFormat::bytes_to_value`'s TOML branch (not just the JSON one already
// covered by `malformed_bytes_surface_as_parse_error` in
// `config_versioning.rs`).
#[test]
fn toml_invalid_utf8_bytes_surface_as_parse_error() {
    let backend = Arc::new(InMemoryBackend::with_bytes(vec![0xFF, 0xFE, 0xFD]));
    let store = ConfigStore::<DemoConfig>::new(backend, ConfigFormat::Toml, 1);
    let err = store.load().unwrap_err();
    assert!(matches!(err, ConfigError::Parse { .. }));
}

#[test]
fn toml_invalid_syntax_surfaces_as_parse_error() {
    let backend = Arc::new(InMemoryBackend::with_bytes(b"not = [valid toml".to_vec()));
    let store = ConfigStore::<DemoConfig>::new(backend, ConfigFormat::Toml, 1);
    let err = store.load().unwrap_err();
    assert!(matches!(err, ConfigError::Parse { .. }));
}

// TOML cannot represent a JSON `null` — a config value with an absent
// `Option` field trips `ConfigFormat::value_to_bytes`'s TOML conversion step,
// surfacing as `ConfigError::Serialize` rather than writing a corrupt file.
#[derive(Default, Clone, Serialize, Deserialize)]
struct OptionalFieldConfig {
    schema_version: u32,
    nickname: Option<String>,
}
impl VersionedConfig for OptionalFieldConfig {
    fn schema_version(&self) -> u32 {
        self.schema_version
    }
    fn set_schema_version(&mut self, v: u32) {
        self.schema_version = v;
    }
}

#[test]
fn toml_cannot_represent_a_null_field_and_save_returns_serialize_error() {
    let store = ConfigStore::<OptionalFieldConfig>::new(
        Arc::new(InMemoryBackend::new()),
        ConfigFormat::Toml,
        1,
    );
    let cfg = OptionalFieldConfig {
        schema_version: 1,
        nickname: None,
    };
    let err = store.save(&cfg).unwrap_err();
    assert!(matches!(err, ConfigError::Serialize { .. }));
}

// User story 21/22-adjacent — read-only backend rejects `save` with the
// typed error, and `ConfigBackend::write` is never even reached.
#[test]
fn read_only_backend_rejects_save() {
    let store = store_over(InMemoryBackend::new().read_only());
    let cfg = DemoConfig::default();
    let err = store.save(&cfg).unwrap_err();
    assert!(matches!(err, ConfigError::ReadOnly { .. }));
    assert!(err.to_string().contains("CE003"));
}

// Negative check for the above: an otherwise-identical writable backend must
// accept the same save — proves the rejection above is really driven by
// `supports_write`, not some unrelated failure.
#[test]
fn writable_backend_accepts_the_same_save() {
    let store = store_over(InMemoryBackend::new());
    let cfg = DemoConfig::default();
    store.save(&cfg).unwrap();
}

// User stories 16-17 — reload() picks up an out-of-band write and notifies a
// subscriber; without reload(), the previously resolved value is unchanged.
#[test]
fn reload_picks_up_out_of_band_write_and_notifies_subscriber() {
    let backend = Arc::new(InMemoryBackend::new());
    let store = ConfigStore::<DemoConfig>::new(backend.clone(), ConfigFormat::default(), 1);
    store.resolve().unwrap();
    assert_eq!(store.current().greeting, "");

    let seen = Arc::new(Mutex::new(Vec::<String>::new()));
    let seen_clone = seen.clone();
    store.subscribe(move |cfg| seen_clone.lock().unwrap().push(cfg.greeting.clone()));

    // Simulate an out-of-band write (e.g. another process, or a hand edit).
    let out_of_band =
        serde_json::json!({"schema_version": 1, "greeting": "from-elsewhere", "count": 0});
    backend.set_bytes(serde_json::to_vec(&out_of_band).unwrap());

    // Without reload(), the cached value is untouched.
    assert_eq!(store.current().greeting, "");

    store.reload().unwrap();
    assert_eq!(store.current().greeting, "from-elsewhere");
    assert_eq!(
        seen.lock().unwrap().as_slice(),
        ["from-elsewhere".to_string()]
    );
}

// Regression: a subscriber callback that itself calls `subscribe()` must not
// deadlock. `reload()` used to invoke callbacks while still holding the
// subscribers lock; `subscribe()` locks the same mutex, so a subscriber
// re-entering the store from within its own callback would hang forever
// (std::sync::Mutex is not reentrant) rather than error. A true deadlock
// can't be asserted directly — it never returns — so this drives `reload()`
// on a background thread and fails if it doesn't complete within a generous
// timeout, which is the standard shape for this kind of regression test.
#[test]
fn reentrant_subscribe_from_within_a_callback_does_not_deadlock() {
    let backend = Arc::new(InMemoryBackend::new());
    let store = Arc::new(ConfigStore::<DemoConfig>::new(
        backend,
        ConfigFormat::default(),
        1,
    ));
    store.resolve().unwrap();

    let inner_saw = Arc::new(Mutex::new(false));
    let inner_saw_clone = inner_saw.clone();
    let store_for_callback = store.clone();
    store.subscribe(move |_cfg| {
        // Re-enter the store from inside a callback — this is the case that
        // used to deadlock.
        let inner_saw_inner = inner_saw_clone.clone();
        store_for_callback.subscribe(move |_cfg| {
            *inner_saw_inner.lock().unwrap() = true;
        });
    });

    let store_for_thread = store.clone();
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let result = store_for_thread.reload();
        let _ = tx.send(result);
    });

    let result = rx
        .recv_timeout(std::time::Duration::from_secs(5))
        .expect("reload() did not return within 5s — reentrant subscribe deadlocked");
    result.unwrap();

    // The re-entrantly-registered subscriber is live for the *next* reload,
    // proving subscribe() from within a callback isn't merely non-deadlocking
    // but actually took effect.
    store.reload().unwrap();
    assert!(*inner_saw.lock().unwrap());
}

// `resolve()` is what `AppBuilder::build()` calls once; a caller that never
// calls `reload()` afterward keeps a stable value even if the backend
// changes underneath it — the one-shot CLI contract (user story 18).
#[test]
fn resolve_caches_but_does_not_auto_refresh() {
    let backend = Arc::new(InMemoryBackend::new());
    let store = ConfigStore::<DemoConfig>::new(backend.clone(), ConfigFormat::default(), 1);
    store.resolve().unwrap();

    let mut cfg = (*store.current()).clone();
    cfg.greeting = "first".to_string();
    store.save(&cfg).unwrap();
    assert_eq!(store.current().greeting, "first");

    // Out-of-band change after the fact; `current()` must not silently move.
    let out_of_band = serde_json::json!({"schema_version": 1, "greeting": "second", "count": 0});
    backend.set_bytes(serde_json::to_vec(&out_of_band).unwrap());
    assert_eq!(
        store.current().greeting,
        "first",
        "current() must stay put without an explicit reload()"
    );
}

// `ConfigStore` is `Send + Sync` — required so a background task and a UI
// thread can both read configuration (user story 23), and so it can be held
// behind an `Arc<dyn ConfigHandle>` in `DispatchEnv`.
#[test]
fn config_store_is_send_and_sync() {
    fn assert_send_sync<T: Send + Sync>() {}
    assert_send_sync::<ConfigStore<DemoConfig>>();
}

// ConfigError implements std::error::Error (matches ProjectConfigError /
// SecretError / AuthError conventions in this crate).
#[test]
fn config_error_is_std_error() {
    fn assert_error<E: std::error::Error>() {}
    assert_error::<ConfigError>();
}

// `ConfigError::backend_read` / `backend_write` are the convenience
// constructors non-filesystem backends (e.g. the Windows registry backend)
// use to wrap an arbitrary underlying error without matching on `io::Error`
// directly.
#[test]
fn backend_read_and_write_helper_constructors() {
    let read_err = ConfigError::backend_read("some-backend", "boom");
    assert!(matches!(read_err, ConfigError::BackendRead { .. }));
    assert!(read_err.to_string().contains("CE001"));
    assert!(read_err.to_string().contains("some-backend"));

    let write_err = ConfigError::backend_write("some-backend", "boom");
    assert!(matches!(write_err, ConfigError::BackendWrite { .. }));
    assert!(write_err.to_string().contains("CE002"));
}

// `ConfigStore::format` / `current_version` are plain accessors mirroring
// what was passed to `new` — used by `doctor`-style introspection.
#[test]
fn format_and_current_version_accessors() {
    let store =
        ConfigStore::<DemoConfig>::new(Arc::new(InMemoryBackend::new()), ConfigFormat::Toml, 5);
    assert_eq!(store.format(), ConfigFormat::Toml);
    assert_eq!(store.current_version(), 5);
}

// `save` maps a serialization failure to `ConfigError::Serialize` rather than
// panicking. A `BTreeMap` keyed by a tuple is the reliable way to trigger
// this with `serde_json`: JSON object keys must be strings, and serde_json's
// map-key serializer rejects compound key types outright (unlike, say, a NaN
// float, which serde_json silently serializes as `null`).
#[derive(Default, Clone, Serialize, Deserialize)]
struct UnrepresentableConfig {
    schema_version: u32,
    bad_map: std::collections::BTreeMap<(i32, i32), String>,
}
impl VersionedConfig for UnrepresentableConfig {
    fn schema_version(&self) -> u32 {
        self.schema_version
    }
    fn set_schema_version(&mut self, v: u32) {
        self.schema_version = v;
    }
}

#[test]
fn save_with_unrepresentable_value_returns_serialize_error() {
    let store = ConfigStore::<UnrepresentableConfig>::new(
        Arc::new(InMemoryBackend::new()),
        ConfigFormat::Json,
        1,
    );
    let mut cfg = UnrepresentableConfig {
        schema_version: 1,
        bad_map: std::collections::BTreeMap::new(),
    };
    cfg.bad_map.insert((1, 2), "unrepresentable".to_string());
    let err = store.save(&cfg).unwrap_err();
    assert!(matches!(err, ConfigError::Serialize { .. }));
    assert!(err.to_string().contains("CE005"));
}

// The object-safe `ConfigHandle` trait is what `AppContext::opt_config_handle`
// returns; these tests call it through `&dyn ConfigHandle` explicitly (rather
// than through `ConfigStore`'s own inherent methods) to exercise the trait
// impl itself, including its error-mapping branches.
#[test]
fn config_handle_reload_and_current_json_via_trait_object() {
    let backend = Arc::new(InMemoryBackend::new());
    let store: Arc<ConfigStore<DemoConfig>> = Arc::new(ConfigStore::new(
        backend.clone(),
        ConfigFormat::default(),
        1,
    ));
    let handle: &dyn ConfigHandle = store.as_ref();

    let value = serde_json::json!({"schema_version": 1, "greeting": "via-handle", "count": 9});
    backend.set_bytes(serde_json::to_vec(&value).unwrap());

    handle.reload().unwrap();
    let current = handle.current_json().unwrap();
    assert_eq!(current["greeting"], "via-handle");
    assert_eq!(handle.backend_label(), "in-memory");
}

// `ConfigHandle::current_json` maps a serialization failure to
// `ConfigError::Serialize` too, distinct from `save_json`'s deserialize-side
// `Parse` error. Deliberately asymmetric (de)serialization on this type is
// what makes it reachable: `load`/`reload` only ever need `Deserialize`
// (which behaves normally here), so a value can land in `current()` that
// this hand-written `Serialize` impl then refuses to re-emit — exactly the
// shape `current_json`'s error-mapping closure exists to catch.
#[derive(Default, Clone, Deserialize)]
struct WriteOnlyFailsConfig {
    schema_version: u32,
    note: String,
}
impl serde::Serialize for WriteOnlyFailsConfig {
    fn serialize<S>(&self, _serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        Err(serde::ser::Error::custom("deliberately unserializable"))
    }
}
impl VersionedConfig for WriteOnlyFailsConfig {
    fn schema_version(&self) -> u32 {
        self.schema_version
    }
    fn set_schema_version(&mut self, v: u32) {
        self.schema_version = v;
    }
}

#[test]
fn config_handle_current_json_maps_serialize_failure() {
    let backend = Arc::new(InMemoryBackend::with_bytes(
        serde_json::to_vec(&serde_json::json!({"schema_version": 1, "note": "x"})).unwrap(),
    ));
    let store: Arc<ConfigStore<WriteOnlyFailsConfig>> =
        Arc::new(ConfigStore::new(backend, ConfigFormat::default(), 1));
    // `resolve` only needs `Deserialize`, so this succeeds even though the
    // type can never be serialized back out.
    let resolved = store.resolve().unwrap();
    assert_eq!(resolved.note, "x");

    let handle: &dyn ConfigHandle = store.as_ref();
    let err = handle.current_json().unwrap_err();
    assert!(matches!(err, ConfigError::Serialize { .. }));
}

#[test]
fn config_handle_save_json_round_trips_and_rejects_malformed_value() {
    let store: Arc<ConfigStore<DemoConfig>> = Arc::new(ConfigStore::new(
        Arc::new(InMemoryBackend::new()),
        ConfigFormat::default(),
        1,
    ));
    let handle: &dyn ConfigHandle = store.as_ref();

    handle
        .save_json(
            serde_json::json!({"schema_version": 0, "greeting": "saved-via-handle", "count": 3}),
        )
        .unwrap();
    assert_eq!(store.current().greeting, "saved-via-handle");
    assert_eq!(store.current().count, 3);

    // A JSON value that cannot deserialize into `DemoConfig` (wrong type for
    // `count`) surfaces as a typed Parse error, not a panic, and does not
    // disturb the previously saved value.
    let err = handle
        .save_json(
            serde_json::json!({"schema_version": 0, "greeting": "x", "count": "not-a-number"}),
        )
        .unwrap_err();
    assert!(matches!(err, ConfigError::Parse { .. }));
    assert_eq!(store.current().greeting, "saved-via-handle");
}

// `InMemoryBackend::default()` and `with_label` — small builder surface
// otherwise only reachable indirectly.
#[test]
fn in_memory_backend_default_and_with_label() {
    let backend = InMemoryBackend::default();
    assert_eq!(backend.label(), "in-memory");
    assert!(backend.read().unwrap().is_empty());

    let labeled = InMemoryBackend::new().with_label("custom-label");
    assert_eq!(labeled.label(), "custom-label");
}

// `InMemoryBackend::write` enforces read-only itself too (defense in depth
// alongside `ConfigStore::save`'s own check) — called directly here so the
// backend's own guard, not just the store's, is exercised.
#[test]
fn in_memory_backend_write_directly_enforces_read_only() {
    let backend = InMemoryBackend::new().read_only();
    let err = backend.write(b"x").unwrap_err();
    assert!(matches!(err, ConfigError::ReadOnly { .. }));
}
