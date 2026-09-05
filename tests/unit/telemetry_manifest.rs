// tests/unit/telemetry_manifest.rs
use cli_framework::config::manifest::{ConfigManifest, FieldKind, FieldManifest, Scope};
use cli_framework::telemetry::{
    merge_telemetry_section, telemetry_only_manifest, telemetry_section, ManifestMergeError,
    ProbeRegistry,
};

fn registry() -> ProbeRegistry {
    ProbeRegistry::with_builtins()
}

fn leaf(key: &str, kind: FieldKind) -> FieldManifest {
    FieldManifest {
        key: key.to_string(),
        kind,
        default: None,
        label: None,
        description: None,
        group: None,
        scope: Scope::Machine,
        platforms: Vec::new(),
        secret: false,
        local_only: false,
        protected: false,
        manageable: true,
        enforceable: true,
        restart_required: false,
        constraints: None,
    }
}

#[test]
fn the_section_declares_the_five_settings_plus_one_switch_per_probe() {
    let manifest = telemetry_only_manifest("demo", &registry(), None);
    let paths: Vec<String> = manifest.iter_leaves().into_iter().map(|l| l.path).collect();

    for expected in [
        "telemetry.level",
        "telemetry.attribution",
        "telemetry.endpoint",
        "telemetry.install_id",
        "telemetry.notice_shown",
    ] {
        assert!(
            paths.contains(&expected.to_string()),
            "missing {expected} in {paths:?}"
        );
    }
    assert!(paths.contains(&"telemetry.cli.command.enabled".to_string()));
    assert!(
        !paths.contains(&"telemetry.enabled".to_string()),
        "there is no global telemetry.enabled key; the telemetry level is the switch"
    );
    assert_eq!(
        paths.iter().filter(|p| p.ends_with(".enabled")).count(),
        registry().len(),
        "one enabled switch per registered probe, no more"
    );
}

#[test]
fn the_telemetry_level_may_be_recommended_but_never_enforced() {
    let manifest = telemetry_only_manifest("demo", &registry(), None);
    let level = manifest.leaf_by_path("telemetry.level").unwrap();
    assert!(
        level.manageable,
        "an organisation may recommend a telemetry level"
    );
    assert!(
        !level.enforceable,
        "an organisation may never enforce a telemetry level: consent cannot be mandated"
    );
    assert!(level.restart_required);
    assert_eq!(level.scope, Scope::Machine);
    match &level.kind {
        FieldKind::Enum { values } => {
            assert_eq!(values, &["off", "usage", "diagnostic", "debug"]);
        }
        other => panic!("expected an enumeration, got {other:?}"),
    }
}

#[test]
fn attribution_is_recommendable_but_not_enforceable_either() {
    let manifest = telemetry_only_manifest("demo", &registry(), None);
    let attribution = manifest.leaf_by_path("telemetry.attribution").unwrap();
    assert!(attribution.manageable);
    assert!(!attribution.enforceable);
    match &attribution.kind {
        FieldKind::Enum { values } => {
            assert_eq!(values, &["anonymous", "pseudonymous", "identified"]);
        }
        other => panic!("expected an enumeration, got {other:?}"),
    }
}

#[test]
fn the_endpoint_is_the_one_telemetry_setting_an_organisation_may_enforce() {
    let manifest = telemetry_only_manifest("demo", &registry(), Some("http://collector:4318"));
    let endpoint = manifest.leaf_by_path("telemetry.endpoint").unwrap();
    assert!(endpoint.manageable);
    assert!(
        endpoint.enforceable,
        "where telemetry goes is the fleet's decision"
    );
    assert_eq!(
        endpoint.default.as_ref().and_then(|v| v.as_str()),
        Some("http://collector:4318")
    );
    assert!(!endpoint.secret, "an endpoint is not a credential");
}

#[test]
fn an_endpointless_app_gets_an_empty_default_which_means_no_export() {
    let manifest = telemetry_only_manifest("demo", &registry(), None);
    let endpoint = manifest.leaf_by_path("telemetry.endpoint").unwrap();
    assert_eq!(endpoint.default.as_ref().and_then(|v| v.as_str()), Some(""));
}

#[test]
fn the_install_id_and_the_notice_record_never_leave_the_machine() {
    let manifest = telemetry_only_manifest("demo", &registry(), None);
    for path in ["telemetry.install_id", "telemetry.notice_shown"] {
        let field = manifest.leaf_by_path(path).unwrap();
        assert!(field.local_only, "{path} must never be set remotely");
        assert!(
            field.protected,
            "{path} must not be writable by an automated caller"
        );
        assert!(
            !field.manageable,
            "{path} is not an organisation's business"
        );
    }
}

#[test]
fn every_probe_switch_is_a_boolean_defaulting_to_on() {
    let manifest = telemetry_only_manifest("demo", &registry(), None);
    let switch = manifest
        .leaf_by_path("telemetry.cli.command.args.enabled")
        .unwrap();
    assert!(matches!(switch.kind, FieldKind::Bool));
    assert_eq!(switch.default, Some(serde_json::Value::Bool(true)));
    assert!(switch.manageable);
    assert!(
        switch.enforceable,
        "an organisation may switch a single probe off for good"
    );
    assert!(
        switch.description.is_some(),
        "a person reading the config surface must be told what the probe sends"
    );
}

#[test]
fn the_section_merges_into_an_apps_own_manifest_without_disturbing_it() {
    let app = ConfigManifest::new("demo", vec![leaf("retries", FieldKind::Int)]);
    let merged = merge_telemetry_section(app, &registry(), None).unwrap();
    let paths: Vec<String> = merged.iter_leaves().into_iter().map(|l| l.path).collect();
    assert!(paths.contains(&"retries".to_string()));
    assert!(paths.contains(&"telemetry.level".to_string()));
    assert_eq!(merged.app, "demo");
}

#[test]
fn an_app_that_already_owns_a_top_level_telemetry_key_is_a_build_error() {
    let app = ConfigManifest::new(
        "demo",
        vec![leaf("telemetry", FieldKind::Section { fields: vec![] })],
    );
    let err = merge_telemetry_section(app, &registry(), None).unwrap_err();
    assert!(
        matches!(err, ManifestMergeError::AppOwnsTelemetryKey),
        "two telemetry trees would publish two contradictory answers; got {err:?}"
    );
    assert!(
        err.to_string().contains("telemetry"),
        "the error must name the colliding key: {err}"
    );
}

#[test]
fn the_section_is_a_section_named_telemetry() {
    let section = telemetry_section(&registry(), None);
    assert_eq!(section.key, "telemetry");
    assert!(matches!(section.kind, FieldKind::Section { .. }));
}
