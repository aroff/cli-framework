// tests/unit/telemetry_store.rs
use cli_framework::config::{ConfigFormat, VersionedConfig};
use cli_framework::telemetry::{
    Attribution, StoreState, TelemetryLevel, TelemetrySettings, TelemetryStore,
};

fn temp_dir(tag: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "cli-fw-telemetry-{}-{}-{}",
        tag,
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn a_fresh_store_reports_defaults_and_no_stored_choice() {
    let dir = temp_dir("fresh");
    let store = TelemetryStore::open_at(&dir, "demo");
    assert!(store.state().is_ready());
    let settings = store.settings();
    assert_eq!(settings.level, None, "a fresh Install has made no choice");
    assert_eq!(settings.attribution, None);
    assert_eq!(settings.notice_shown, None);
    assert!(settings.probes.is_empty());
}

#[test]
fn the_settings_live_in_the_apps_own_directory_under_a_telemetry_file() {
    let dir = temp_dir("path");
    let store = TelemetryStore::open_at(&dir, "demo");
    store
        .mutate(|s| s.level = Some(TelemetryLevel::Usage))
        .unwrap();
    let written = dir.join("demo").join("telemetry.json");
    assert!(written.is_file(), "expected {} to exist", written.display());
    let raw = std::fs::read_to_string(&written).unwrap();
    assert!(raw.contains("\"usage\""), "got: {raw}");
    assert!(raw.contains("\"schema_version\""), "got: {raw}");
}

#[test]
fn a_stored_choice_survives_reopening_the_store() {
    let dir = temp_dir("persist");
    TelemetryStore::open_at(&dir, "demo")
        .mutate(|s| {
            s.level = Some(TelemetryLevel::Diagnostic);
            s.attribution = Some(Attribution::Anonymous);
            s.probes.insert("cli.command.args".into(), false);
        })
        .unwrap();

    let reopened = TelemetryStore::open_at(&dir, "demo").settings();
    assert_eq!(reopened.level, Some(TelemetryLevel::Diagnostic));
    assert_eq!(reopened.attribution, Some(Attribution::Anonymous));
    assert_eq!(reopened.probes.get("cli.command.args"), Some(&false));
    assert_eq!(reopened.schema_version, 1);
}

#[test]
fn the_install_id_is_minted_once_and_then_reused() {
    let dir = temp_dir("install-id");
    let first = TelemetryStore::open_at(&dir, "demo")
        .ensure_install_id()
        .unwrap();
    let second = TelemetryStore::open_at(&dir, "demo")
        .ensure_install_id()
        .unwrap();
    assert_eq!(first, second, "reopening must not mint a second Install");
    assert_eq!(first.len(), 36, "expected a UUID v4: {first}");
    assert_eq!(&first[14..15], "4", "expected a version-4 UUID: {first}");
}

#[test]
fn reset_clears_the_stored_choice_but_keeps_the_install_id() {
    let dir = temp_dir("reset");
    let store = TelemetryStore::open_at(&dir, "demo");
    let id = store.ensure_install_id().unwrap();
    store
        .mutate(|s| {
            s.level = Some(TelemetryLevel::Debug);
            s.notice_shown = Some(TelemetryLevel::Debug);
        })
        .unwrap();

    store.reset().unwrap();

    let after = TelemetryStore::open_at(&dir, "demo").settings();
    assert_eq!(after.level, None);
    assert_eq!(after.notice_shown, None);
    assert_eq!(
        after.install_id,
        Some(id),
        "reset returns the Install to no-choice; it does not make it a new Install"
    );
}

#[test]
fn a_store_with_no_usable_directory_is_unavailable_and_names_the_reason() {
    let dir = temp_dir("blocked");
    let blocker = dir.join("demo");
    std::fs::write(&blocker, b"not a directory").unwrap();

    let store = TelemetryStore::open_at(&dir, "demo");
    match store.state() {
        StoreState::Unavailable(reason) => {
            assert!(!reason.is_empty(), "an unavailable store must say why");
        }
        StoreState::Ready(p) => panic!("expected the store to be unavailable, got {p:?}"),
    }
    assert!(
        store
            .mutate(|s| s.level = Some(TelemetryLevel::Usage))
            .is_err(),
        "writing to an unavailable store must fail loudly, not silently succeed"
    );
    assert_eq!(store.ensure_install_id(), None);
    assert_eq!(
        store.settings().level,
        None,
        "reading it must still not panic"
    );
}

#[test]
fn a_corrupt_settings_file_reads_as_defaults_rather_than_failing_startup() {
    let dir = temp_dir("corrupt");
    std::fs::create_dir_all(dir.join("demo")).unwrap();
    std::fs::write(dir.join("demo").join("telemetry.json"), b"{ not json").unwrap();

    let store = TelemetryStore::open_at(&dir, "demo");
    assert_eq!(store.settings().level, None);
}

#[test]
fn state_reason_and_describe_distinguish_ready_from_unavailable() {
    let dir = temp_dir("state-reporting");
    let ready = TelemetryStore::open_at(&dir, "demo");
    assert_eq!(ready.state().reason(), None, "a ready store has no reason");
    let expected_path = dir.join("demo").join("telemetry.json");
    assert_eq!(
        ready.state().describe(),
        expected_path.display().to_string(),
        "a ready store's one-line description is its file path"
    );

    let blocker_dir = temp_dir("state-reporting-blocked");
    std::fs::write(blocker_dir.join("demo"), b"not a directory").unwrap();
    let unavailable = TelemetryStore::open_at(&blocker_dir, "demo");
    let reason = unavailable
        .state()
        .reason()
        .expect("an unavailable store must report why through reason()");
    assert!(!reason.is_empty());
    assert!(
        unavailable.state().describe().starts_with("unavailable: "),
        "got: {}",
        unavailable.state().describe()
    );
}

#[test]
fn versioned_config_impl_reads_and_writes_the_schema_version_field() {
    let mut settings = TelemetrySettings {
        schema_version: 3,
        ..Default::default()
    };
    assert_eq!(
        VersionedConfig::schema_version(&settings),
        3,
        "the getter must read the same field mutate()/load() stamp"
    );
    settings.set_schema_version(9);
    assert_eq!(
        settings.schema_version, 9,
        "the setter must write the field the getter reads"
    );
}

#[test]
fn open_resolves_a_real_platform_config_directory_when_one_exists() {
    // Every other test uses `open_at()` against a temp directory so it stays
    // hermetic. This is the only one exercising the actual `open()` entry
    // point production code calls, which goes through `dirs::config_dir()`.
    // Some sandboxes have no resolvable config directory at all, so both of
    // `StoreState`'s variants are accepted here — but if one is created, it
    // is removed again so the smoke test leaves nothing behind.
    let app = "cli-framework-telemetry-store-open-smoke-test";
    let store = TelemetryStore::open(app);
    match store.state() {
        StoreState::Ready(path) => {
            assert!(
                path.ends_with("telemetry.json"),
                "expected a telemetry.json path, got {path:?}"
            );
            let _ = std::fs::remove_dir_all(path.parent().unwrap());
        }
        StoreState::Unavailable(reason) => {
            assert!(!reason.is_empty());
        }
    }
}

#[test]
fn a_toml_app_stores_its_telemetry_settings_as_toml_beside_it() {
    // PRD 025: the settings file's extension follows the application's own
    // configuration format. A TOML app must not get a lone JSON file in a
    // directory its user hand-edits.
    let dir = temp_dir("toml-app");
    let store = TelemetryStore::open_at_with_format(&dir, "demo", ConfigFormat::Toml);
    let path = dir.join("demo").join("telemetry.toml");
    assert_eq!(store.state(), &StoreState::Ready(path.clone()));

    store
        .mutate(|s| {
            s.level = Some(TelemetryLevel::Diagnostic);
            s.attribution = Some(Attribution::Anonymous);
        })
        .expect("a ready toml store must be writable");

    let raw = std::fs::read_to_string(&path).expect("the toml file must exist");
    let parsed: toml::Value = toml::from_str(&raw).expect("the bytes on disk must be TOML");
    assert_eq!(parsed["level"].as_str(), Some("diagnostic"));
    assert!(
        serde_json::from_str::<serde_json::Value>(&raw).is_err(),
        "TOML was expected on disk, but the bytes also parse as JSON: {raw}"
    );
    assert!(
        !dir.join("demo").join("telemetry.json").exists(),
        "a toml app must not leave a stray telemetry.json behind"
    );

    let reopened = TelemetryStore::open_at_with_format(&dir, "demo", ConfigFormat::Toml);
    assert_eq!(reopened.settings().level, Some(TelemetryLevel::Diagnostic));
    assert_eq!(
        reopened.settings().attribution,
        Some(Attribution::Anonymous)
    );
}

#[test]
fn the_two_argument_constructors_are_the_json_default() {
    // `open_at`/`open` are the "app declares no configuration" case, so they
    // must stay byte-for-byte the JSON variant rather than drifting from it.
    let dir = temp_dir("json-default");
    let defaulted = TelemetryStore::open_at(&dir, "demo");
    assert_eq!(
        defaulted.state(),
        TelemetryStore::open_at_with_format(&dir, "demo", ConfigFormat::Json).state(),
        "open_at must be exactly the JSON format variant"
    );

    defaulted
        .mutate(|s| s.level = Some(TelemetryLevel::Usage))
        .expect("a ready json store must be writable");
    let raw = std::fs::read_to_string(dir.join("demo").join("telemetry.json"))
        .expect("the json file must exist");
    let parsed: serde_json::Value =
        serde_json::from_str(&raw).expect("the bytes on disk must be JSON");
    assert_eq!(parsed["level"].as_str(), Some("usage"));
}
