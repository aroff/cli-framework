// tests/unit/telemetry_env.rs
use cli_framework::telemetry::{
    env_var_name, scan_environment, telemetry_only_manifest, ProbeRegistry,
};

fn vars(pairs: &[(&str, &str)]) -> Vec<(String, String)> {
    pairs
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect()
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
