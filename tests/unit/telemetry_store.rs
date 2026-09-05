// tests/unit/telemetry_store.rs
use cli_framework::telemetry::{Attribution, StoreState, TelemetryLevel, TelemetryStore};

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
