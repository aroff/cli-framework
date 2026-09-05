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

/// Assert `os.version` is either absent (a platform this crate cannot answer
/// for) or a real, coarse version.
///
/// Two things are being pinned. The value must look like a version — the
/// defect this replaced reported `std::env::consts::FAMILY`, i.e. the literal
/// `"unix"`, under a semantic-convention key, and every `is_some()` oracle in
/// the suite was happy with it. And it must be *coarse*: at most
/// `major.minor`, because a full build number (`6.8.0-136-generic`) is close
/// to a fingerprint on a small population, which is exactly what the
/// specification's identity rules exist to prevent.
fn assert_os_version_is_a_version(value: Option<&str>) {
    #[cfg(target_os = "linux")]
    let value = Some(value.expect(
        "on Linux the kernel release is readable from /proc, so os.version must be present",
    ));

    let Some(value) = value else { return };

    let parts: Vec<&str> = value.split('.').collect();
    assert!(
        parts.len() <= 2,
        "os.version must be coarsened to at most major.minor, got {value:?}"
    );
    assert!(
        parts
            .iter()
            .all(|p| !p.is_empty() && p.chars().all(|c| c.is_ascii_digit())),
        "os.version must be numeric components only, got {value:?}"
    );
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
    // Not `is_some()`. The version has to be the *SDK's*, and the defect this
    // replaced — `env!("CARGO_PKG_VERSION")`, which expands to cli-framework's
    // own version — passed an existence check happily while telling every
    // backend that `opentelemetry` was at whatever version this crate had
    // released. Assert the two are different, and that the value looks like a
    // version at all.
    let sdk_version = value(&attrs, "telemetry.sdk.version")
        .expect("the SDK triple must be present on the metric resource");
    assert_ne!(
        sdk_version,
        env!("CARGO_PKG_VERSION"),
        "telemetry.sdk.version must describe the OpenTelemetry SDK, not cli-framework"
    );
    assert!(
        sdk_version
            .split('.')
            .next()
            .is_some_and(|major| major.chars().all(|c| c.is_ascii_digit())),
        "telemetry.sdk.version should look like a version, got {sdk_version:?}"
    );
    assert_eq!(value(&attrs, "telemetry.sdk.name"), Some("opentelemetry"));
    assert_eq!(value(&attrs, "telemetry.sdk.language"), Some("rust"));
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
    // A reader needs the lock too. This test calls the two builders in two
    // separate statements and then asserts they agree; both read
    // `OTEL_SERVICE_NAME`, so a concurrent test setting or clearing it between
    // them makes the equality loop fail on a variable this test never mentions.
    // Marking only the writers is the classic half-serialized suite.
    let _service_name = EnvGuard::unset("OTEL_SERVICE_NAME");
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

    // Proof the guard above is actually held, and not merely written down:
    // with `OTEL_SERVICE_NAME` cleared for the duration, `service.name` must
    // fall back to the application's own name whatever the ambient
    // environment says. Without the guard this reads whatever the shell — or
    // a concurrent test in this binary — happens to have set.
    assert_eq!(
        value(&metric, "service.name"),
        Some(service().name.as_str()),
        "the guard must have cleared OTEL_SERVICE_NAME for the whole of this test"
    );

    for (k, v) in &metric {
        assert_eq!(
            value(&trace, k),
            Some(v.as_str()),
            "{k} must be identical on both resources"
        );
    }
    assert_eq!(value(&trace, "cli.install.id"), Some("install-1"));
    assert_eq!(value(&trace, "session.id"), Some("session-1"));
    // `os.version` is present only where this crate can obtain a real one
    // without a new dependency, so the assertion is conditional on the
    // platform rather than on the attribute — a bare `is_some()` was satisfied
    // by the placeholder `"unix"` that this replaced.
    assert_os_version_is_a_version(value(&trace, "os.version"));
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

#[test]
fn the_sdk_triple_describes_the_opentelemetry_sdk_and_not_this_crate() {
    // The three `telemetry.sdk.*` attributes come from the SDK's own resource
    // detector rather than being written out here, so they cannot drift when
    // the dependency is bumped. The trap they replaced was subtle: hand-writing
    // `telemetry.sdk.version` as `env!("CARGO_PKG_VERSION")` compiles, is
    // present, is a plausible version string, and is wrong — it is
    // cli-framework's version sitting under a key that says `opentelemetry`.
    let policy = policy_with(Deployment::Service, TelemetryLevel::Diagnostic, |_| {});
    let attrs = metric_resource_attrs(&policy, &service());

    assert_eq!(value(&attrs, "telemetry.sdk.name"), Some("opentelemetry"));
    assert_eq!(value(&attrs, "telemetry.sdk.language"), Some("rust"));

    let sdk_version = value(&attrs, "telemetry.sdk.version").expect("sdk version must be present");
    assert_ne!(
        sdk_version,
        env!("CARGO_PKG_VERSION"),
        "reporting this crate's version as the SDK's is the defect this test exists for"
    );
    assert_ne!(
        sdk_version,
        service().version,
        "nor may it be the *application's* version"
    );
    assert!(
        sdk_version.split('.').count() >= 2
            && sdk_version
                .split('.')
                .all(|p| p.chars().next().is_some_and(|c| c.is_ascii_digit())),
        "telemetry.sdk.version should be a semver-shaped SDK version, got {sdk_version:?}"
    );
}

#[test]
fn os_version_is_coarsened_to_at_most_major_and_minor() {
    // Tested through the pure helper rather than only through the resource,
    // because the truncation is the part carrying the privacy requirement and
    // it must not be reachable solely on the platform that happens to have a
    // /proc to read. A precise kernel build is a near-identifier on a small
    // population; `6.8` is not.
    use cli_framework::telemetry::coarse_version_for_test as coarse;

    assert_eq!(coarse("6.8.0-136-generic"), "6.8");
    assert_eq!(coarse("6.8.0"), "6.8");
    assert_eq!(coarse("5.15.0-1071-azure\n"), "5.15");
    assert_eq!(coarse("10.0.19045"), "10.0");
    // A single component is a version too; there is nothing to truncate.
    assert_eq!(coarse("7"), "7");
    // Nothing numeric at the front means nothing this crate is willing to
    // publish — better an absent attribute than an invented one.
    assert_eq!(coarse("darwin-arm64"), "");
    assert_eq!(coarse(""), "");
    assert_eq!(coarse("   "), "");
}

#[test]
fn the_trace_resource_omits_os_version_rather_than_inventing_one() {
    // Whatever this platform can answer, the attribute is either absent or a
    // genuine coarse version — never a placeholder standing in for one.
    let policy = policy_with(
        Deployment::EndUser { privacy_url: None },
        TelemetryLevel::Usage,
        |p| p.install_id = Some("install-9".into()),
    );
    let trace = trace_resource_attrs(&policy, &service());
    let os_version = value(&trace, "os.version");

    assert_os_version_is_a_version(os_version);
    if let Some(v) = os_version {
        assert_ne!(
            v,
            std::env::consts::FAMILY,
            "the OS *family* is not a version; it is also strictly coarser than the os.type \
             already on this resource"
        );
        assert_ne!(v, std::env::consts::OS, "nor is the OS name a version");
    }
}
