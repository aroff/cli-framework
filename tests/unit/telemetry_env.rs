// tests/unit/telemetry_env.rs
use cli_framework::config::manifest::{ConfigManifest, FieldKind, FieldManifest, Scope};
use cli_framework::telemetry::{
    env_var_name, scan_environment, telemetry_only_manifest, ProbeRegistry,
};

fn vars(pairs: &[(&str, &str)]) -> Vec<(String, String)> {
    pairs
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect()
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

/// A manifest whose `telemetry` section has exactly one leaf of `kind`, so a
/// single variable can be scanned against a [`FieldKind`] the real telemetry
/// section never declares itself (there is no built-in integer, duration or
/// float leaf under `telemetry.` — `typed()` still must handle them, because
/// nothing stops an application's own manifest fields from living there once
/// merged, and `scan_environment` walks every leaf under the prefix).
fn manifest_with_custom_leaf(kind: FieldKind) -> ConfigManifest {
    ConfigManifest::new(
        "demo",
        vec![leaf(
            "telemetry",
            FieldKind::Section {
                fields: vec![leaf("custom", kind)],
            },
        )],
    )
}

#[test]
fn a_dotted_path_becomes_an_underscored_upper_case_variable() {
    assert_eq!(
        env_var_name("demo", "telemetry.level"),
        "DEMO_TELEMETRY_LEVEL"
    );
    assert_eq!(
        env_var_name("demo-app", "telemetry.cli.command.args.enabled"),
        "DEMO_APP_TELEMETRY_CLI_COMMAND_ARGS_ENABLED"
    );
}

#[test]
fn a_matching_variable_lands_in_the_environment_layer_typed() {
    let manifest = telemetry_only_manifest("demo", &ProbeRegistry::with_builtins(), None);
    let scan = scan_environment(
        "demo",
        &manifest,
        &vars(&[
            ("DEMO_TELEMETRY_LEVEL", "diagnostic"),
            ("DEMO_TELEMETRY_CLI_COMMAND_ENABLED", "false"),
        ]),
    );
    assert_eq!(scan.values.get("telemetry.level").unwrap(), "diagnostic");
    assert_eq!(
        scan.values.get("telemetry.cli.command.enabled").unwrap(),
        &serde_json::Value::Bool(false),
        "a boolean field must arrive as a JSON boolean, not the string \"false\""
    );
    assert!(scan.unmatched.is_empty());
}

#[test]
fn a_misspelt_telemetry_variable_is_reported_rather_than_ignored() {
    let manifest = telemetry_only_manifest("demo", &ProbeRegistry::with_builtins(), None);
    let scan = scan_environment(
        "demo",
        &manifest,
        &vars(&[("DEMO_TELEMETRY_LEVELS", "usage")]),
    );
    assert!(scan.values.is_empty());
    assert_eq!(scan.unmatched, vec!["DEMO_TELEMETRY_LEVELS".to_string()]);
}

#[test]
fn variables_belonging_to_other_applications_or_subtrees_are_left_alone() {
    let manifest = telemetry_only_manifest("demo", &ProbeRegistry::with_builtins(), None);
    let scan = scan_environment(
        "demo",
        &manifest,
        &vars(&[
            ("OTHER_TELEMETRY_LEVEL", "debug"),
            ("DEMO_RETRIES", "5"),
            ("PATH", "/usr/bin"),
        ]),
    );
    assert!(scan.values.is_empty());
    assert!(
        scan.unmatched.is_empty(),
        "only unmatched <APP>_TELEMETRY_* variables are worth a warning: {:?}",
        scan.unmatched
    );
}

#[test]
fn an_empty_value_is_taken_literally_not_treated_as_unset() {
    let manifest = telemetry_only_manifest("demo", &ProbeRegistry::with_builtins(), None);
    let scan = scan_environment("demo", &manifest, &vars(&[("DEMO_TELEMETRY_ENDPOINT", "")]));
    assert_eq!(scan.values.get("telemetry.endpoint").unwrap(), "");
}

#[test]
fn the_kill_switch_variable_is_not_an_unmatched_setting() {
    let manifest = telemetry_only_manifest("demo", &ProbeRegistry::with_builtins(), None);
    let scan = scan_environment(
        "demo",
        &manifest,
        &vars(&[("DEMO_TELEMETRY_DISABLED", "1")]),
    );
    assert!(
        scan.unmatched.is_empty(),
        "the kill switch is handled before resolution; it is not a typo"
    );
}

#[test]
fn an_integer_shaped_value_for_a_string_field_stays_a_string() {
    let manifest = telemetry_only_manifest("demo", &ProbeRegistry::with_builtins(), None);
    let scan = scan_environment(
        "demo",
        &manifest,
        &vars(&[("DEMO_TELEMETRY_INSTALL_ID", "12345")]),
    );
    assert_eq!(
        scan.values.get("telemetry.install_id").unwrap(),
        "12345",
        "the field's declared kind decides the JSON type, not the value's appearance"
    );
}

#[test]
fn an_integer_shaped_variable_for_an_integer_field_becomes_a_json_number() {
    let manifest = manifest_with_custom_leaf(FieldKind::Int);
    let scan = scan_environment("demo", &manifest, &vars(&[("DEMO_TELEMETRY_CUSTOM", "42")]));
    assert_eq!(
        scan.values.get("telemetry.custom").unwrap(),
        &serde_json::Value::Number(42.into())
    );
}

#[test]
fn a_duration_field_is_parsed_as_a_whole_number_of_seconds() {
    let manifest = manifest_with_custom_leaf(FieldKind::Duration);
    let scan = scan_environment("demo", &manifest, &vars(&[("DEMO_TELEMETRY_CUSTOM", "30")]));
    assert_eq!(
        scan.values.get("telemetry.custom").unwrap(),
        &serde_json::Value::Number(30.into()),
        "duration shares the integer parser: it is whole seconds on the wire"
    );
}

#[test]
fn a_float_shaped_variable_for_a_float_field_becomes_a_json_number() {
    let manifest = manifest_with_custom_leaf(FieldKind::Float);
    let scan = scan_environment(
        "demo",
        &manifest,
        &vars(&[("DEMO_TELEMETRY_CUSTOM", "0.5")]),
    );
    let value = scan.values.get("telemetry.custom").unwrap();
    assert_eq!(value.as_f64(), Some(0.5));
}

#[test]
fn an_unrecognized_value_for_a_boolean_field_is_left_as_a_string_not_guessed_at() {
    let manifest = manifest_with_custom_leaf(FieldKind::Bool);
    let scan = scan_environment(
        "demo",
        &manifest,
        &vars(&[("DEMO_TELEMETRY_CUSTOM", "maybe")]),
    );
    assert_eq!(
        scan.values.get("telemetry.custom").unwrap(),
        &serde_json::Value::String("maybe".to_string()),
        "typed() must not guess when a boolean-shaped variable isn't one of the recognized words"
    );
}
