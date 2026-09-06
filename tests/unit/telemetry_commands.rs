// tests/unit/telemetry_commands.rs
//! Unit tests for the pure `telemetry` command functions (Task 22).
//!
//! These call `status_report`/`set_level`/`disable_probe`/`enable_probe`/
//! `reset` directly — no `AppBuilder`, no CLI dispatch. The end-to-end
//! wiring (a real app, a real `telemetry` subcommand, a real exit code) is
//! `tests/integration/telemetry_cli.rs`'s job instead.
//!
//! Two fixups relative to the plan text, both required by the real struct
//! definitions in `src/telemetry/policy.rs`:
//! - `TelemetryPolicy::level_source` is `Layer`, not `String` — so a test
//!   that wants to name the layer a level came from sets
//!   `p.level_source = Layer::ConfigFile` rather than a bare string.
//! - `TelemetryPolicy::kill_switch` is `Option<KillSwitch>`, not
//!   `Option<String>` — so a test naming an active kill switch sets
//!   `p.kill_switch = Some(KillSwitch::DoNotTrack)`.
//! `StatusReport`'s own fields stay `String`/`Option<String>` (see
//! `commands.rs`'s `layer_label`/`KillSwitch::as_str` conversions), so the
//! JSON-facing assertions below are unaffected and still compare to plain
//! string literals.
//!
//! `TelemetryStore`'s own methods take `&self`, so `temp_store()`'s `mut`
//! bindings from the plan are dropped: `set_level`/`disable_probe`/
//! `enable_probe`/`reset` all take `&TelemetryStore` here, not `&mut`.
//! `TelemetryStore::settings()` returns `TelemetrySettings` directly (there
//! is no fallible `read()`), so `store.read().unwrap()` becomes
//! `store.settings()`.

use cli_framework::config::resolution::Layer;
use cli_framework::telemetry::{
    disable_probe, enable_probe, info_catalog, reset, set_level, status_report, Attribution,
    Deployment, KillSwitch, SetOutcome, TelemetryLevel,
};

mod support;
use support::{
    policy_with, ready_store, registry, temp_store, temp_store_with_kill_switch, unavailable_store,
    EnvGuard,
};

fn json_keys(value: &serde_json::Value) -> Vec<String> {
    let mut keys: Vec<String> = value.as_object().unwrap().keys().cloned().collect();
    keys.sort();
    keys
}

#[test]
fn the_json_status_field_set_is_exactly_the_documented_contract() {
    let policy = policy_with(
        Deployment::EndUser { privacy_url: None },
        TelemetryLevel::Usage,
        |_| {},
    );
    let report = status_report(&policy, &ready_store());
    let value = serde_json::to_value(&report).unwrap();
    let mut expected = vec![
        "level",
        "level_source",
        "attribution",
        "install_id",
        "endpoint",
        "endpoint_source",
        "policy",
        "kill_switch",
        "probes",
        "store",
    ];
    expected.sort();
    assert_eq!(
        json_keys(&value),
        expected,
        "scripts parse this; adding or renaming a field is a deliberate act, \
         not an accident"
    );
}

#[test]
fn the_status_names_the_layer_the_level_came_from() {
    let policy = policy_with(
        Deployment::EndUser { privacy_url: None },
        TelemetryLevel::Usage,
        |p| {
            p.level_source = Layer::ConfigFile;
        },
    );
    let report = status_report(&policy, &ready_store());
    assert_eq!(report.level_source, "config_file");
}

#[test]
fn an_anonymous_install_reports_a_null_install_id_rather_than_omitting_the_field() {
    let policy = policy_with(
        Deployment::EndUser { privacy_url: None },
        TelemetryLevel::Usage,
        |p| {
            p.attribution = Attribution::Anonymous;
            p.install_id = None;
        },
    );
    let value = serde_json::to_value(status_report(&policy, &ready_store())).unwrap();
    assert!(value["install_id"].is_null());
    assert!(
        value.as_object().unwrap().contains_key("install_id"),
        "a missing key and a null value mean different things to a script"
    );
}

#[test]
fn an_active_kill_switch_is_named_in_the_status() {
    let policy = policy_with(
        Deployment::EndUser { privacy_url: None },
        TelemetryLevel::Off,
        |p| {
            p.kill_switch = Some(KillSwitch::DoNotTrack);
        },
    );
    let value = serde_json::to_value(status_report(&policy, &ready_store())).unwrap();
    assert_eq!(value["kill_switch"], "DO_NOT_TRACK");
    assert_eq!(value["level"], "off");
}

#[test]
fn every_probe_appears_in_the_status_with_its_effective_state() {
    let policy = policy_with(
        Deployment::EndUser { privacy_url: None },
        TelemetryLevel::Usage,
        |_| {},
    );
    let report = status_report(&policy, &ready_store());
    assert_eq!(
        report.probes.len(),
        policy.registry.len(),
        "a probe missing from status is a probe nobody can audit"
    );
    let command = report
        .probes
        .iter()
        .find(|p| p.id == "cli.command")
        .unwrap();
    assert!(
        command.effective,
        "cli.command is a usage probe at usage level"
    );
    let args = report
        .probes
        .iter()
        .find(|p| p.id == "cli.command.args")
        .unwrap();
    assert!(!args.effective, "cli.command.args needs diagnostic");
    assert_eq!(args.min_level, "diagnostic", "and the status says why");
}

#[test]
fn info_catalog_reports_enabled_and_effective_exactly_as_status_does() {
    let policy = policy_with(
        Deployment::EndUser { privacy_url: None },
        TelemetryLevel::Usage,
        |p| {
            p.disabled_probes.insert("cli.command".to_string());
        },
    );
    let catalog = info_catalog(&policy);
    assert_eq!(
        catalog.len(),
        policy.registry.len(),
        "info must list every probe in the registry, not a hand-picked subset"
    );
    let command = catalog.iter().find(|p| p.id == "cli.command").unwrap();
    assert!(!command.enabled, "cli.command was explicitly disabled");
    assert!(
        !command.effective,
        "a disabled probe is never effective regardless of level"
    );
    let args = catalog.iter().find(|p| p.id == "cli.command.args").unwrap();
    assert!(args.enabled, "cli.command.args itself was never disabled");
    assert!(
        !args.effective,
        "cli.command.args needs diagnostic; usage level leaves it ineffective"
    );
    assert_eq!(
        args.min_level, "diagnostic",
        "and info says why, same as status"
    );
}

#[test]
fn info_lists_the_full_catalog_even_when_the_store_is_unavailable() {
    // The one true thing in the old "the catalog is a property of the binary"
    // rationale: `info` must still work before an install has configured
    // anything, i.e. when the store could not even be opened. Wire a real
    // `unavailable_store()` the same way `build_policy` does
    // (`store_available: store.state().is_ready()`,
    // `store_error: store.state().reason()...`) and confirm `info_catalog`
    // does not drop a single probe because of it.
    let store = unavailable_store("config directory could not be created");
    let policy = policy_with(
        Deployment::EndUser { privacy_url: None },
        TelemetryLevel::Off,
        |p| {
            p.store_available = store.state().is_ready();
            p.store_error = store.state().reason().map(str::to_string);
        },
    );
    assert!(
        !policy.store_available,
        "the fixture must actually simulate an unavailable store"
    );
    let catalog = info_catalog(&policy);
    assert_eq!(
        catalog.len(),
        policy.registry.len(),
        "the catalog must be the full registry regardless of store health"
    );
    for probe in ["cli.command", "cli.panic", "http.client", "mcp.session"] {
        assert!(
            catalog.iter().any(|p| p.id == probe),
            "{probe} missing from info's catalog when the store is unavailable"
        );
    }
}

#[test]
fn setting_a_level_writes_it_and_reports_it_applied() {
    // `set_level`'s own kill-switch check reads the real process environment
    // (`commands.rs` calls `detect_kill_switch(app_name, ...)`), the same
    // ambient state
    // `setting_a_level_that_a_kill_switch_overrides_...` mutates below via
    // `temp_store_with_kill_switch`. Rust's default test harness runs both in
    // parallel threads of one process, so without this guard the two race:
    // this test can observe the other's `OTEL_SDK_DISABLED=true` and get
    // `AppliedButClamped` instead of `Applied` — reproduced empirically.
    // `EnvGuard::unset` both serializes against every other `EnvGuard`-
    // holding test in this binary (one process-wide lock, held for the
    // guard's life) and guarantees the variable is actually absent for the
    // duration, regardless of what the ambient shell exported before
    // `cargo test` ran.
    let _guard = EnvGuard::unset("OTEL_SDK_DISABLED");
    let (store, _dir) = temp_store();
    assert_eq!(
        set_level(&store, "demo", TelemetryLevel::Usage).unwrap(),
        SetOutcome::Applied
    );
    assert_eq!(store.settings().level, Some(TelemetryLevel::Usage));
}

#[test]
fn setting_a_level_that_a_kill_switch_overrides_says_so_instead_of_reporting_success() {
    let (store, _dir, _guard) = temp_store_with_kill_switch("OTEL_SDK_DISABLED");
    match set_level(&store, "demo", TelemetryLevel::Debug).unwrap() {
        SetOutcome::AppliedButClamped { effective, reason } => {
            assert_eq!(effective, TelemetryLevel::Off);
            assert!(reason.contains("OTEL_SDK_DISABLED"));
        }
        SetOutcome::Applied => panic!(
            "the value was stored but has no effect; reporting plain success \
             would be a lie a person acts on"
        ),
    }
}

#[test]
fn setting_a_level_with_no_writable_store_fails_with_the_reason() {
    let store = unavailable_store("config directory could not be created");
    let err = set_level(&store, "demo", TelemetryLevel::Usage).unwrap_err();
    assert!(
        err.to_string().contains("config directory"),
        "a mutating command must fail loudly when the store is unavailable — \
         silently discarding a consent change is the worst possible outcome"
    );
}

#[test]
fn disabling_a_probe_that_does_not_exist_is_an_error_not_a_silent_no_op() {
    let (store, _dir) = temp_store();
    let err = disable_probe(&store, &registry(), "cli.nonexistent").unwrap_err();
    assert!(err.to_string().contains("cli.nonexistent"));
}

#[test]
fn disabling_a_parent_probe_disables_its_children_without_writing_them() {
    let (store, _dir) = temp_store();
    disable_probe(&store, &registry(), "cli.command").unwrap();
    let stored = store.settings();
    assert_eq!(stored.probes.get("cli.command"), Some(&false));
    assert!(
        !stored.probes.contains_key("cli.command.args"),
        "the subtree rule lives in the resolver, not in the stored document — \
         writing children would freeze today's catalog into a person's config file"
    );
}

#[test]
fn enabling_a_probe_that_does_not_exist_is_an_error_not_a_silent_no_op() {
    let (store, _dir) = temp_store();
    let err = enable_probe(&store, &registry(), "cli.nonexistent").unwrap_err();
    assert!(err.to_string().contains("cli.nonexistent"));
}

#[test]
fn enabling_a_previously_disabled_probe_removes_its_stored_override() {
    let (store, _dir) = temp_store();
    disable_probe(&store, &registry(), "cli.command").unwrap();
    assert_eq!(store.settings().probes.get("cli.command"), Some(&false));
    enable_probe(&store, &registry(), "cli.command").unwrap();
    assert!(
        !store.settings().probes.contains_key("cli.command"),
        "enabling clears the stored override rather than writing `true` back — \
         the resolver already treats \"absent\" as enabled by default, and a \
         cleared override is what lets a later policy change re-decide the \
         probe instead of an old choice sticking forever"
    );
}

#[test]
fn an_enforced_organisation_policy_is_named_in_the_status() {
    let policy = policy_with(
        Deployment::EndUser { privacy_url: None },
        TelemetryLevel::Usage,
        |p| {
            p.level_source = Layer::Enforced;
        },
    );
    let report = status_report(&policy, &ready_store());
    assert_eq!(report.policy, "organisation policy: enforced");
}

#[test]
fn reset_removes_the_stored_choices_but_never_the_install_id() {
    let (store, _dir) = temp_store();
    set_level(&store, "demo", TelemetryLevel::Debug).unwrap();
    let id_before = store.settings().install_id.clone();
    reset(&store).unwrap();
    let after = store.settings();
    assert_eq!(after.level, None);
    assert_eq!(
        after.install_id, id_before,
        "reset returns to defaults; minting a new identifier would make one \
         person look like two installs in every dashboard"
    );
}

#[test]
fn setting_a_level_sees_the_app_prefixed_kill_switch_not_only_the_generic_ones() {
    // `<APP>_TELEMETRY_DISABLED` is the switch `detect_kill_switch` tests
    // *first* and the only one whose name depends on the application. A
    // `set_level` that passed no app name would skip it entirely and report
    // plain success, while `telemetry status` on the very same install
    // reported telemetry off — two commands disagreeing about one machine.
    let _guard = EnvGuard::set("DEMO_TELEMETRY_DISABLED", "1");
    let (store, _dir) = temp_store();
    match set_level(&store, "demo", TelemetryLevel::Debug).unwrap() {
        SetOutcome::AppliedButClamped { effective, reason } => {
            assert_eq!(effective, TelemetryLevel::Off);
            assert!(
                reason.contains("DEMO_TELEMETRY_DISABLED"),
                "the reason has to name the variable the person must unset, \
                 expanded for this app; got: {reason}"
            );
            assert!(
                !reason.contains("<APP>"),
                "an unexpanded placeholder is not a variable anyone can \
                 search their shell profile for; got: {reason}"
            );
        }
        SetOutcome::Applied => panic!(
            "DEMO_TELEMETRY_DISABLED=1 is set, so the stored level has no \
             effect; reporting plain success is the lie this test exists to \
             catch"
        ),
    }
}

#[test]
fn the_status_names_the_kill_switch_variable_expanded_for_this_app() {
    let mut policy = policy_with(
        Deployment::EndUser { privacy_url: None },
        TelemetryLevel::Off,
        |p| p.kill_switch = Some(KillSwitch::AppDisabled),
    );
    policy.app = "demo-app".to_string();
    let report = status_report(&policy, &ready_store());
    assert_eq!(
        report.kill_switch.as_deref(),
        Some("DEMO_APP_TELEMETRY_DISABLED"),
        "status is where a person goes to find out why telemetry is off; \
         printing the literal `<APP>_TELEMETRY_DISABLED` tells them to unset \
         a variable that does not exist"
    );
}
