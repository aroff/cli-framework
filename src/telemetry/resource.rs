//! The two OpenTelemetry Resources (ADR 0079).
//!
//! A Resource is attached to every signal a provider emits, so what goes on it
//! is decided once and paid for on every point. That makes the metric Resource
//! and the trace Resource genuinely different objects rather than a nicety:
//!
//! * A **metric** Resource is part of every time series' identity. An
//!   installation identifier there would create one series per installation,
//!   which is both an unbounded-cardinality bill and a per-person metric store.
//!   Resource A therefore describes the *shape* of an install — which app, which
//!   version, which deployment, which telemetry level, which OS and
//!   architecture — and identifies nobody.
//! * A **trace** is already a record of one invocation. Correlating it to an
//!   installation is what pseudonymous attribution is for, so Resource B is A
//!   plus `cli.install.id`, `session.id` and `os.version`.
//!
//! Neither Resource ever carries a host name, a host id, a user name, or a
//! command line. On an end-user machine a host name is frequently a person's
//! name; on a server it is a fleet inventory nobody asked us to publish.

use super::policy::TelemetryPolicy;

/// What the application calls itself.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServiceIdentity {
    pub name: String,
    pub version: String,
}

fn push(attrs: &mut Vec<(String, String)>, key: &str, value: impl Into<String>) {
    attrs.push((key.to_string(), value.into()));
}

/// Resource A — attached to the meter provider.
pub fn metric_resource_attrs(
    policy: &TelemetryPolicy,
    service: &ServiceIdentity,
) -> Vec<(String, String)> {
    let mut attrs = Vec::with_capacity(9);
    // OTEL_SERVICE_NAME wins over the app name: the platform injects it, and
    // an operator setting it is making a deliberate naming decision.
    push(
        &mut attrs,
        "service.name",
        std::env::var("OTEL_SERVICE_NAME")
            .ok()
            .filter(|v| !v.trim().is_empty())
            .unwrap_or_else(|| service.name.clone()),
    );
    push(&mut attrs, "service.version", service.version.clone());
    push(&mut attrs, "cli.deployment", policy.deployment.as_str());
    push(&mut attrs, "cli.telemetry.level", policy.level.as_str());
    push(&mut attrs, "os.type", std::env::consts::OS);
    push(&mut attrs, "host.arch", std::env::consts::ARCH);
    push(&mut attrs, "telemetry.sdk.language", "rust");
    push(&mut attrs, "telemetry.sdk.name", "opentelemetry");
    push(
        &mut attrs,
        "telemetry.sdk.version",
        env!("CARGO_PKG_VERSION"),
    );
    attrs
}

/// Resource B — attached to the tracer provider (and, later, the logger).
pub fn trace_resource_attrs(
    policy: &TelemetryPolicy,
    service: &ServiceIdentity,
) -> Vec<(String, String)> {
    let mut attrs = metric_resource_attrs(policy, service);
    if let Some(id) = &policy.install_id {
        push(&mut attrs, "cli.install.id", id.clone());
    }
    push(&mut attrs, "session.id", policy.session_id.clone());
    push(&mut attrs, "os.version", os_version());
    attrs
}

/// A coarse OS version. Deliberately coarse: a precise build number is close
/// to a fingerprint on a small population.
fn os_version() -> String {
    std::env::consts::FAMILY.to_string()
}

/// Apply `OTEL_RESOURCE_ATTRIBUTES`, honoured on both Resources.
///
/// An operator who sets `deployment.environment` expects it everywhere, and an
/// operator who overrides `service.name` means it. Malformed entries are
/// skipped rather than failing startup — a typo in one entry must not take
/// telemetry down.
pub fn apply_env_resource_attributes(attrs: &mut Vec<(String, String)>, raw: Option<&str>) {
    let Some(raw) = raw else { return };
    for entry in raw.split(',') {
        let Some((key, value)) = entry.split_once('=') else {
            continue;
        };
        let key = key.trim();
        if key.is_empty() {
            continue;
        }
        let value = value.trim().to_string();
        match attrs.iter_mut().find(|(k, _)| k == key) {
            Some(existing) => existing.1 = value,
            None => attrs.push((key.to_string(), value)),
        }
    }
}

/// Convert to the SDK type at the call site.
pub fn to_resource(attrs: Vec<(String, String)>) -> opentelemetry_sdk::Resource {
    opentelemetry_sdk::Resource::builder_empty()
        .with_attributes(
            attrs
                .into_iter()
                .map(|(k, v)| opentelemetry::KeyValue::new(k, v))
                .collect::<Vec<_>>(),
        )
        .build()
}
