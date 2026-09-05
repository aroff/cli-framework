// tests/unit/telemetry_resource.rs
use cli_framework::telemetry::{
    apply_env_resource_attributes, metric_resource_attrs, trace_resource_attrs, Attribution,
    Deployment, ServiceIdentity, TelemetryLevel,
};

mod support;
use support::{policy_with, EnvGuard};

fn service() -> ServiceIdentity {
    ServiceIdentity {
        name: "demo".to_string(),
        version: "1.2.3".to_string(),
    }
}

fn keys(attrs: &[(String, String)]) -> Vec<&str> {
    attrs.iter().map(|(k, _)| k.as_str()).collect()
}

fn value<'a>(attrs: &'a [(String, String)], key: &str) -> Option<&'a str> {
    attrs
        .iter()
        .find(|(k, _)| k == key)
        .map(|(_, v)| v.as_str())
}

#[test]
fn the_metric_resource_carries_no_identifier_at_all() {
    let policy = policy_with(
        Deployment::EndUser { privacy_url: None },
        TelemetryLevel::Usage,
        |p| {
            p.attribution = Attribution::Pseudonymous;
            p.install_id = Some("11111111-2222-3333-4444-555555555555".into());
            p.session_id = "session-abc".into();
        },
    );
    let attrs = metric_resource_attrs(&policy, &service());
    assert!(
        !keys(&attrs).contains(&"cli.install.id"),
        "an install id on a metric resource mints one time series per installation: {:?}",
        keys(&attrs)
    );
    assert!(!keys(&attrs).contains(&"session.id"));
    assert!(!keys(&attrs).contains(&"enduser.id"));
}

#[test]
fn the_platform_can_name_the_service_through_the_environment() {
    // The collector chart injects OTEL_SERVICE_NAME alongside the endpoint
    // (spec 025 §11). An operator who sets it means it, and it is how one
    // binary deployed twice becomes two services in a dashboard.
    let _g = EnvGuard::set("OTEL_SERVICE_NAME", "billing-api");
    let policy = policy_with(Deployment::Service, TelemetryLevel::Diagnostic, |_| {});
    let attrs = metric_resource_attrs(&policy, &service());
    assert_eq!(value(&attrs, "service.name"), Some("billing-api"));
}

#[test]
fn without_the_environment_variable_the_service_name_is_the_app_name() {
    let _g = EnvGuard::unset("OTEL_SERVICE_NAME");
    let policy = policy_with(Deployment::Service, TelemetryLevel::Diagnostic, |_| {});
    assert_eq!(
        value(&metric_resource_attrs(&policy, &service()), "service.name"),
        Some("demo")
    );
}

#[test]
fn the_metric_resource_carries_the_shape_of_the_install_not_the_install() {
    // This test's `service.name` assertion below assumes no platform override
    // is in effect. Without this guard the assertion races against
    // `the_platform_can_name_the_service_through_the_environment`, which sets
    // `OTEL_SERVICE_NAME` in the same process — `cargo test` runs `#[test]`
    // functions concurrently by default, and the environment is process-wide,
    // not per-thread.
    let _g = EnvGuard::unset("OTEL_SERVICE_NAME");
    let policy = policy_with(Deployment::Service, TelemetryLevel::Diagnostic, |_| {});
    let attrs = metric_resource_attrs(&policy, &service());
    assert_eq!(value(&attrs, "service.name"), Some("demo"));
    assert_eq!(value(&attrs, "service.version"), Some("1.2.3"));
    assert_eq!(value(&attrs, "cli.deployment"), Some("service"));
    assert_eq!(value(&attrs, "cli.telemetry.level"), Some("diagnostic"));
    assert!(value(&attrs, "os.type").is_some());
    assert!(value(&attrs, "host.arch").is_some());
    assert_eq!(value(&attrs, "telemetry.sdk.language"), Some("rust"));
    assert_eq!(value(&attrs, "telemetry.sdk.name"), Some("opentelemetry"));
    assert!(value(&attrs, "telemetry.sdk.version").is_some());
}

#[test]
fn the_metric_resource_never_carries_the_host_name() {
    let policy = policy_with(Deployment::Service, TelemetryLevel::Debug, |_| {});
    for attrs in [
        metric_resource_attrs(&policy, &service()),
        trace_resource_attrs(&policy, &service()),
    ] {
        assert!(
            !keys(&attrs).contains(&"host.name"),
            "a host name is a personal identifier on an end-user machine and a \
             fleet inventory on a server: {:?}",
            keys(&attrs)
        );
        assert!(!keys(&attrs).contains(&"host.id"));
        assert!(!keys(&attrs).contains(&"process.command_line"));
    }
}

#[test]
fn the_trace_resource_is_the_metric_resource_plus_the_identity_triple() {
    let policy = policy_with(
        Deployment::EndUser { privacy_url: None },
        TelemetryLevel::Usage,
        |p| {
            p.install_id = Some("install-1".into());
            p.session_id = "session-1".into();
        },
    );
    let metric = metric_resource_attrs(&policy, &service());
    let trace = trace_resource_attrs(&policy, &service());

    for (k, v) in &metric {
        assert_eq!(
            value(&trace, k),
            Some(v.as_str()),
            "{k} must be identical on both resources"
        );
    }
    assert_eq!(value(&trace, "cli.install.id"), Some("install-1"));
    assert_eq!(value(&trace, "session.id"), Some("session-1"));
    assert!(value(&trace, "os.version").is_some());
}

#[test]
fn an_anonymous_install_puts_no_install_id_on_the_trace_resource_either() {
    let policy = policy_with(
        Deployment::EndUser { privacy_url: None },
        TelemetryLevel::Usage,
        |p| {
            p.attribution = Attribution::Anonymous;
            p.install_id = None;
        },
    );
    let trace = trace_resource_attrs(&policy, &service());
    assert!(!keys(&trace).contains(&"cli.install.id"));
    assert!(
        value(&trace, "session.id").is_some(),
        "a session id is per-process and does not survive the process; it is not an identifier \
         of a person and stays even when anonymous"
    );
}

#[test]
fn the_standard_resource_attributes_variable_is_honoured_on_both_resources() {
    let mut attrs = vec![("service.name".to_string(), "demo".to_string())];
    apply_env_resource_attributes(
        &mut attrs,
        Some("deployment.environment=prod,team=platform"),
    );
    assert_eq!(value(&attrs, "deployment.environment"), Some("prod"));
    assert_eq!(value(&attrs, "team"), Some("platform"));
    assert_eq!(value(&attrs, "service.name"), Some("demo"));
}

#[test]
fn an_environment_supplied_attribute_may_override_a_framework_one() {
    let mut attrs = vec![("service.name".to_string(), "demo".to_string())];
    apply_env_resource_attributes(&mut attrs, Some("service.name=renamed"));
    assert_eq!(value(&attrs, "service.name"), Some("renamed"));
    assert_eq!(attrs.len(), 1, "override, not duplicate: {attrs:?}");
}

#[test]
fn a_malformed_resource_attributes_entry_is_skipped_not_fatal() {
    let mut attrs = Vec::new();
    apply_env_resource_attributes(&mut attrs, Some("good=1,nonsense,=novalue,also=fine"));
    assert_eq!(value(&attrs, "good"), Some("1"));
    assert_eq!(value(&attrs, "also"), Some("fine"));
    assert_eq!(attrs.len(), 2, "got {attrs:?}");
}
