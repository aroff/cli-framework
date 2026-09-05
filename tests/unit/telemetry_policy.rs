// tests/unit/telemetry_policy.rs
use cli_framework::config::resolution::Layer;
use cli_framework::telemetry::{
    detect_kill_switch, resolve_policy, Attribution, Deployment, KillSwitch, LayeredLevel,
    ProbeRegistry, TelemetryInputs, TelemetryLevel,
};
use std::collections::HashMap;

fn env_of(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> {
    let map: HashMap<String, String> = pairs
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect();
    move |k: &str| map.get(k).cloned()
}

fn inputs(deployment: Deployment) -> TelemetryInputs {
    TelemetryInputs {
        app: "demo".into(),
        deployment,
        registry: ProbeRegistry::with_builtins(),
        session_id: "s".into(),
        store_available: true,
        ..Default::default()
    }
}

#[test]
fn each_of_the_three_kill_switches_is_detected() {
    assert_eq!(
        detect_kill_switch("demo-app", &env_of(&[("DEMO_APP_TELEMETRY_DISABLED", "1")])),
        Some(KillSwitch::AppDisabled)
    );
    assert_eq!(
        detect_kill_switch("demo-app", &env_of(&[("OTEL_SDK_DISABLED", "true")])),
        Some(KillSwitch::OtelSdkDisabled)
    );
    assert_eq!(
        detect_kill_switch("demo-app", &env_of(&[("DO_NOT_TRACK", "1")])),
        Some(KillSwitch::DoNotTrack)
    );
    assert_eq!(detect_kill_switch("demo-app", &env_of(&[])), None);
}

#[test]
fn a_kill_switch_variable_set_to_anything_else_does_not_fire() {
    assert_eq!(
        detect_kill_switch("demo", &env_of(&[("DEMO_TELEMETRY_DISABLED", "0")])),
        None
    );
    assert_eq!(
        detect_kill_switch("demo", &env_of(&[("OTEL_SDK_DISABLED", "false")])),
        None
    );
}

#[test]
fn an_end_user_install_starts_off() {
    let policy = resolve_policy(inputs(Deployment::default()));
    assert_eq!(policy.level, TelemetryLevel::Off);
    assert_eq!(policy.level_source, Layer::Default);
    assert!(!policy.exports());
}

#[test]
fn a_service_with_an_endpoint_starts_at_diagnostic() {
    let mut i = inputs(Deployment::Service);
    i.endpoint = Some("http://collector:4318".into());
    let policy = resolve_policy(i);
    assert_eq!(policy.level, TelemetryLevel::Diagnostic);
    assert!(policy.exports());
}

#[test]
fn a_service_without_an_endpoint_stays_off() {
    let policy = resolve_policy(inputs(Deployment::Service));
    assert_eq!(policy.level, TelemetryLevel::Off);
    assert!(!policy.exports());
}

#[test]
fn on_an_end_user_install_the_environment_cannot_raise_the_telemetry_level() {
    let mut i = inputs(Deployment::default());
    i.endpoint = Some("http://collector:4318".into());
    i.level = LayeredLevel {
        environment: Some(TelemetryLevel::Debug),
        ..Default::default()
    };
    let policy = resolve_policy(i);
    assert_eq!(
        policy.level,
        TelemetryLevel::Off,
        "the environment must not be able to switch telemetry on behind a person's back"
    );
}

#[test]
fn on_an_end_user_install_a_builder_override_cannot_raise_the_telemetry_level() {
    let mut i = inputs(Deployment::default());
    i.endpoint = Some("http://collector:4318".into());
    i.level = LayeredLevel {
        builder_override: Some(TelemetryLevel::Usage),
        ..Default::default()
    };
    assert_eq!(resolve_policy(i).level, TelemetryLevel::Off);
}

#[test]
fn on_an_end_user_install_the_environment_may_still_lower_the_telemetry_level() {
    let mut i = inputs(Deployment::default());
    i.endpoint = Some("http://collector:4318".into());
    i.level = LayeredLevel {
        config_file: Some(TelemetryLevel::Debug),
        environment: Some(TelemetryLevel::Usage),
        ..Default::default()
    };
    let policy = resolve_policy(i);
    assert_eq!(policy.level, TelemetryLevel::Usage);
}

#[test]
fn an_organisation_recommendation_may_raise_an_end_user_install_that_never_chose() {
    let mut i = inputs(Deployment::default());
    i.endpoint = Some("http://collector:4318".into());
    i.level = LayeredLevel {
        recommended: Some(TelemetryLevel::Usage),
        ..Default::default()
    };
    let policy = resolve_policy(i);
    assert_eq!(policy.level, TelemetryLevel::Usage);
    assert_eq!(policy.level_source, Layer::Recommended);
}

#[test]
fn a_stored_choice_beats_an_organisation_recommendation() {
    let mut i = inputs(Deployment::default());
    i.endpoint = Some("http://collector:4318".into());
    i.level = LayeredLevel {
        recommended: Some(TelemetryLevel::Diagnostic),
        config_file: Some(TelemetryLevel::Off),
        ..Default::default()
    };
    let policy = resolve_policy(i);
    assert_eq!(policy.level, TelemetryLevel::Off);
    assert_eq!(policy.level_source, Layer::ConfigFile);
}

#[test]
fn a_service_is_not_clamped_and_the_environment_wins() {
    let mut i = inputs(Deployment::Service);
    i.endpoint = Some("http://collector:4318".into());
    i.level = LayeredLevel {
        environment: Some(TelemetryLevel::Debug),
        ..Default::default()
    };
    let policy = resolve_policy(i);
    assert_eq!(policy.level, TelemetryLevel::Debug);
    assert_eq!(policy.level_source, Layer::Environment);
}

#[test]
fn a_kill_switch_beats_every_layer_including_a_stored_choice() {
    let mut i = inputs(Deployment::Service);
    i.endpoint = Some("http://collector:4318".into());
    i.level = LayeredLevel {
        config_file: Some(TelemetryLevel::Debug),
        ..Default::default()
    };
    i.kill_switch = Some(KillSwitch::DoNotTrack);
    let policy = resolve_policy(i);
    assert_eq!(policy.level, TelemetryLevel::Off);
    assert!(!policy.exports());
}

#[test]
fn export_needs_both_a_telemetry_level_above_off_and_an_endpoint() {
    let mut i = inputs(Deployment::Service);
    i.level = LayeredLevel {
        builder_override: Some(TelemetryLevel::Usage),
        ..Default::default()
    };
    assert!(!resolve_policy(i).exports(), "no endpoint means no export");
}

#[test]
fn an_unavailable_store_degrades_attribution_to_anonymous_and_drops_the_install_id() {
    let mut i = inputs(Deployment::default());
    i.store_available = false;
    i.store_error = Some("no config directory".into());
    i.attribution = Attribution::Identified;
    i.install_id = Some("11111111-1111-4111-8111-111111111111".into());
    let policy = resolve_policy(i);
    assert_eq!(policy.attribution, Attribution::Anonymous);
    assert_eq!(policy.install_id, None);
}

#[test]
fn probe_effectiveness_reads_through_the_policy() {
    let mut i = inputs(Deployment::Service);
    i.endpoint = Some("http://collector:4318".into());
    i.disabled_probes.insert("cli.command".into());
    let policy = resolve_policy(i);
    assert!(policy.effective("cli.process"));
    assert!(!policy.effective("cli.command"));
    assert!(
        !policy.effective("cli.command.args"),
        "a disabled parent disables the subtree through the policy too"
    );
}

#[test]
fn the_sampler_is_always_on_for_end_user_installs_and_at_debug() {
    let mut end_user = inputs(Deployment::default());
    end_user.endpoint = Some("http://c:4318".into());
    assert!(resolve_policy(end_user).sampler_is_always_on());

    let mut service = inputs(Deployment::Service);
    service.endpoint = Some("http://c:4318".into());
    assert!(!resolve_policy(service.clone()).sampler_is_always_on());

    service.level = LayeredLevel {
        environment: Some(TelemetryLevel::Debug),
        ..Default::default()
    };
    assert!(resolve_policy(service).sampler_is_always_on());
}
