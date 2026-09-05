//! The one place a telemetry decision is made.
//!
//! [`resolve_policy`] is pure: it takes every input the process has already
//! gathered and returns an immutable [`TelemetryPolicy`]. It builds nothing,
//! reads no file and touches no environment variable, so every rule in the
//! spec — the resolution order, the end-user clamp, the kill switches, the
//! export condition — is testable without a collector.

use super::axes::{Attribution, Deployment, TelemetryLevel};
use super::probe::{self, ProbeRegistry};
use crate::config::resolution::Layer;
use std::collections::BTreeSet;

/// A switch that forces telemetry off before anything else is considered.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KillSwitch {
    /// `<APP>_TELEMETRY_DISABLED=1`
    AppDisabled,
    /// `OTEL_SDK_DISABLED=true`
    OtelSdkDisabled,
    /// `DO_NOT_TRACK=1`
    DoNotTrack,
}

impl KillSwitch {
    /// The variable that fired, for `telemetry status` and the doctor.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::AppDisabled => "<APP>_TELEMETRY_DISABLED",
            Self::OtelSdkDisabled => "OTEL_SDK_DISABLED",
            Self::DoNotTrack => "DO_NOT_TRACK",
        }
    }
}

/// `demo-app` becomes `DEMO_APP`. The same transform the `telemetry.`
/// environment mapping uses for the rest of the subtree.
pub fn env_var_prefix(app: &str) -> String {
    app.to_ascii_uppercase().replace(['-', '.'], "_")
}

/// Check the three kill switches in order. `env` is injected so this is
/// testable without mutating the process environment, which is global state
/// shared by every test in the binary.
pub fn detect_kill_switch(app: &str, env: &dyn Fn(&str) -> Option<String>) -> Option<KillSwitch> {
    let app_var = format!("{}_TELEMETRY_DISABLED", env_var_prefix(app));
    if env(&app_var).as_deref() == Some("1") {
        return Some(KillSwitch::AppDisabled);
    }
    if env("OTEL_SDK_DISABLED").as_deref() == Some("true") {
        return Some(KillSwitch::OtelSdkDisabled);
    }
    if env("DO_NOT_TRACK").as_deref() == Some("1") {
        return Some(KillSwitch::DoNotTrack);
    }
    None
}

/// The telemetry level as each layer of the resolution order supplied it.
///
/// There is deliberately no `flags` field: the spec forbids a command-line
/// flag from setting the telemetry level, so the layer is never populated and
/// an absent field is a stronger guarantee than an unused one.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LayeredLevel {
    pub recommended: Option<TelemetryLevel>,
    pub config_file: Option<TelemetryLevel>,
    pub environment: Option<TelemetryLevel>,
    pub builder_override: Option<TelemetryLevel>,
}

/// Everything the process gathered before a decision is made.
#[derive(Debug, Clone, Default)]
pub struct TelemetryInputs {
    pub app: String,
    pub deployment: Deployment,
    pub level: LayeredLevel,
    pub endpoint: Option<String>,
    pub endpoint_source: Option<Layer>,
    pub attribution: Attribution,
    pub install_id: Option<String>,
    pub session_id: String,
    pub kill_switch: Option<KillSwitch>,
    pub registry: ProbeRegistry,
    pub disabled_probes: BTreeSet<String>,
    pub store_available: bool,
    pub store_error: Option<String>,
}

/// The decision. Immutable, computed once per process, shared through an
/// `Arc` by the sampler, the Resources, the exporter and the end-user surface.
#[derive(Debug, Clone)]
pub struct TelemetryPolicy {
    pub app: String,
    pub deployment: Deployment,
    pub level: TelemetryLevel,
    pub level_source: Layer,
    pub attribution: Attribution,
    pub endpoint: Option<String>,
    pub endpoint_source: Option<Layer>,
    pub install_id: Option<String>,
    pub session_id: String,
    pub kill_switch: Option<KillSwitch>,
    pub registry: ProbeRegistry,
    pub disabled_probes: BTreeSet<String>,
    pub store_available: bool,
    pub store_error: Option<String>,
}

fn fold_layers(
    default: TelemetryLevel,
    layers: &LayeredLevel,
    clamped: bool,
) -> (TelemetryLevel, Layer) {
    let mut current = (default, Layer::Default);
    if let Some(v) = layers.recommended {
        current = (v, Layer::Recommended);
    }
    if let Some(v) = layers.config_file {
        current = (v, Layer::ConfigFile);
    }
    if !clamped {
        if let Some(v) = layers.environment {
            current = (v, Layer::Environment);
        }
        if let Some(v) = layers.builder_override {
            current = (v, Layer::BuilderOverride);
        }
    }
    current
}

/// Fold the inputs into a policy. Pure.
pub fn resolve_policy(inputs: TelemetryInputs) -> TelemetryPolicy {
    let default_level = if inputs.deployment.is_end_user() || inputs.endpoint.is_none() {
        TelemetryLevel::Off
    } else {
        TelemetryLevel::Diagnostic
    };

    let (level, level_source) = if inputs.kill_switch.is_some() {
        (TelemetryLevel::Off, Layer::Enforced)
    } else {
        let full = fold_layers(default_level, &inputs.level, false);
        if inputs.deployment.is_end_user() {
            // The clamp: on an end-user Install, `effective = min(full
            // resolution, resolution without environment/flags/builder
            // overrides)`, so only a stored choice or an organisation
            // recommendation can raise the telemetry level.
            let restricted = fold_layers(default_level, &inputs.level, true);
            if restricted.0 < full.0 {
                restricted
            } else {
                full
            }
        } else {
            full
        }
    };

    let attribution = if inputs.store_available {
        inputs.attribution
    } else {
        Attribution::Anonymous
    };
    let install_id = if attribution == Attribution::Anonymous {
        None
    } else {
        inputs.install_id
    };

    TelemetryPolicy {
        app: inputs.app,
        deployment: inputs.deployment,
        level,
        level_source,
        attribution,
        endpoint: inputs.endpoint,
        endpoint_source: inputs.endpoint_source,
        install_id,
        session_id: inputs.session_id,
        kill_switch: inputs.kill_switch,
        registry: inputs.registry,
        disabled_probes: inputs.disabled_probes,
        store_available: inputs.store_available,
        store_error: inputs.store_error,
    }
}

impl TelemetryPolicy {
    /// Export happens only above `off` and only with an endpoint to send to.
    pub fn exports(&self) -> bool {
        self.kill_switch.is_none() && self.level > TelemetryLevel::Off && self.endpoint.is_some()
    }

    /// Whether one probe may emit under this policy.
    pub fn effective(&self, probe_id: &str) -> bool {
        probe::effective(&self.registry, self.level, probe_id, &|id| {
            !self.disabled_probes.contains(id)
        })
    }

    /// End-user Installs sample everything, because there is one process and
    /// a dropped trace is the whole story. `debug` forces the same.
    pub fn sampler_is_always_on(&self) -> bool {
        self.deployment.is_end_user() || self.level == TelemetryLevel::Debug
    }
}
