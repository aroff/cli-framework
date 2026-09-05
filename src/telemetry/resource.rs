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
use opentelemetry_sdk::resource::{ResourceDetector, TelemetryResourceDetector};

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
    // The SDK triple comes from the SDK. Writing it out by hand meant
    // `telemetry.sdk.version` carried *this crate's* version while
    // `telemetry.sdk.name` said `opentelemetry`, so a backend keying
    // behaviour off the pair was told something false, and the value drifted
    // every time cli-framework released. `TelemetryResourceDetector` emits
    // `telemetry.sdk.{name,language,version}` from `opentelemetry_sdk`'s own
    // `CARGO_PKG_VERSION`; an SDK bump now updates it with no edit here.
    for (key, value) in TelemetryResourceDetector.detect().iter() {
        push(&mut attrs, key.as_str(), value.as_str().into_owned());
    }
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
    if let Some(version) = os_version() {
        push(&mut attrs, "os.version", version);
    }
    attrs
}

/// A coarse OS version, or `None` where this crate cannot obtain one.
///
/// Deliberately coarse — `6.8`, not `6.8.0-136-generic`. A precise build
/// number is close to a fingerprint on a small population, and the question
/// this attribute answers ("which OS generation is this cohort on") needs two
/// components at most.
///
/// Returning `None` is the point of the signature. This used to return
/// `std::env::consts::FAMILY` — `unix` or `windows` — which is not a version,
/// is strictly coarser than the `os.type` already on Resource A, and reported
/// something false under a semantic-convention key. An absent attribute is
/// honest; a wrong one is not.
///
/// Linux is the only platform the standard library can answer for: `/proc` is
/// an ordinary file read, while macOS and Windows each need a syscall this
/// crate has no dependency for. See
/// `specs/028-telemetry-os-version-coverage.md`.
fn os_version() -> Option<String> {
    let release = std::fs::read_to_string("/proc/sys/kernel/osrelease").ok()?;
    let coarse = coarse_version(&release);
    (!coarse.is_empty()).then_some(coarse)
}

/// Keep at most the `major.minor` of a version string, and nothing that is not
/// a number.
///
/// Separate from [`os_version`] so it can be tested without a `/proc` — the
/// truncation is the part with the privacy requirement on it, and it must not
/// be reachable only on the platform that happens to have the file.
fn coarse_version(raw: &str) -> String {
    let numeric = |p: &&str| p.chars().all(|c| c.is_ascii_digit());
    let mut parts = raw
        .trim()
        .split(['.', '-', '_'])
        .filter(|p| !p.is_empty())
        .take_while(numeric);
    match (parts.next(), parts.next()) {
        (Some(major), Some(minor)) => format!("{major}.{minor}"),
        (Some(major), None) => major.to_string(),
        _ => String::new(),
    }
}

/// Test-only view of [`coarse_version`], which is private because nothing
/// outside this module should be truncating version strings.
#[doc(hidden)]
pub fn coarse_version_for_test(raw: &str) -> String {
    coarse_version(raw)
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
