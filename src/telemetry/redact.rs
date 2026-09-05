//! What may be recorded, as pure functions of a key and a telemetry level.
//!
//! This is the part of the export boundary that must be auditable line by
//! line, so on purpose it does not touch the SDK and does not touch a policy
//! lookup that could be mocked wrong — it is tested by calling it.
//!
//! Three rules compose, in this order:
//!
//! 1. **Never-list wins over everything.** A key whose name contains
//!    `password`, `secret`, `token`, `authorization`, `cookie` or `api_key` —
//!    case-insensitive substring, plus whatever the author added — is dropped
//!    at every telemetry level including debug. No allowlist entry overrides
//!    it.
//! 2. **A key has a minimum telemetry level.** `exception.message` is
//!    debug-only, `error.type` is diagnostic-and-up, everything else the
//!    framework declares is usage-and-up.
//! 3. **An application attribute needs the author's allowlist.** A
//!    framework-prefixed key is already governed by rule 2 and does not need
//!    it.

use super::axes::TelemetryLevel;
use super::policy::TelemetryPolicy;
use opentelemetry::KeyValue;

pub const NEVER_LIST: &[&str] = &[
    "password",
    "secret",
    "token",
    "authorization",
    "cookie",
    "api_key",
];

pub const METRIC_LABEL_ALLOWLIST: &[&str] = &[
    "command",
    "surface",
    "status",
    "kind",
    "feature",
    "check",
    "severity",
    "tool",
    "plugin",
    "http.route",
    "http.request.method",
    "http.response.status_code",
];

pub const PROBE_ATTR_KEY: &str = "cli.probe";

const FRAMEWORK_PREFIXES: &[&str] = &[
    "cli.",
    "http.",
    "mcp.",
    "otel.",
    "exception.",
    "panic.",
    "error.",
    "session.",
    "service.",
    "telemetry.",
    "url.",
    "rpc.",
    "server.",
];

const FRAMEWORK_BARE_KEYS: &[&str] = &[
    "command",
    "surface",
    "status",
    "kind",
    "feature",
    "check",
    "severity",
    "tool",
    "plugin",
    "duration_ms",
];

const ELEVATED: &[(&str, TelemetryLevel)] = &[
    ("exception.message", TelemetryLevel::Debug),
    ("exception.stacktrace", TelemetryLevel::Debug),
    ("panic.message", TelemetryLevel::Debug),
    ("cli.command.arg_values", TelemetryLevel::Debug),
    ("cli.usage_error.token", TelemetryLevel::Debug),
    ("error.type", TelemetryLevel::Diagnostic),
    ("cli.command.args", TelemetryLevel::Diagnostic),
    ("server.address", TelemetryLevel::Diagnostic),
    ("http.client.server_address", TelemetryLevel::Diagnostic),
];

/// Is this key on the never-list, built-in or author-extended?
pub fn is_never_listed(key: &str, extra: &[String]) -> bool {
    let lowered = key.to_ascii_lowercase();
    NEVER_LIST.iter().any(|f| lowered.contains(f))
        || extra
            .iter()
            .any(|f| lowered.contains(&f.to_ascii_lowercase()))
}

/// The lowest telemetry level at which this key may be recorded.
pub fn attribute_min_level(key: &str) -> TelemetryLevel {
    ELEVATED
        .iter()
        .find(|(k, _)| *k == key)
        .map(|(_, l)| *l)
        .unwrap_or(TelemetryLevel::Usage)
}

/// May this key appear as a metric label?
pub fn metric_label_is_allowed(key: &str) -> bool {
    METRIC_LABEL_ALLOWLIST.contains(&key)
}

/// Read the probe id an instrumentation site declared.
pub fn probe_of(attrs: &[KeyValue]) -> Option<&str> {
    attrs
        .iter()
        .find(|kv| kv.key.as_str() == PROBE_ATTR_KEY)
        .and_then(|kv| match &kv.value {
            opentelemetry::Value::String(s) => Some(s.as_str()),
            _ => None,
        })
}

/// The three rules, bound to one resolved policy.
#[derive(Debug, Clone)]
pub struct RedactionRules {
    pub level: TelemetryLevel,
    /// Application attribute keys the author declared with
    /// `with_telemetry_attrs`.
    pub app_attr_allowlist: Vec<String>,
    /// Extra never-list fragments from `with_telemetry_never`.
    pub extra_never: Vec<String>,
}

impl RedactionRules {
    pub fn from_policy(policy: &TelemetryPolicy) -> Self {
        Self {
            level: policy.level,
            app_attr_allowlist: policy.app_attr_allowlist.clone(),
            extra_never: policy.extra_never.clone(),
        }
    }

    fn is_framework_key(key: &str) -> bool {
        FRAMEWORK_PREFIXES.iter().any(|p| key.starts_with(p)) || FRAMEWORK_BARE_KEYS.contains(&key)
    }

    /// The whole decision for one attribute.
    pub fn keeps_attribute(&self, key: &str) -> bool {
        // 1. Never-list first, so nothing below can override it.
        if is_never_listed(key, &self.extra_never) {
            return false;
        }
        // 2. Off records nothing at all.
        if self.level == TelemetryLevel::Off {
            return false;
        }
        // 3. Application attributes need the author's allowlist.
        if !Self::is_framework_key(key) && !self.app_attr_allowlist.iter().any(|a| a == key) {
            return false;
        }
        // 4. Framework keys carry a minimum telemetry level.
        self.level >= attribute_min_level(key)
    }

    /// Drop everything [`keeps_attribute`](Self::keeps_attribute) rejects,
    /// in place.
    pub fn retain_attributes(&self, attrs: &mut Vec<KeyValue>) {
        attrs.retain(|kv| self.keeps_attribute(kv.key.as_str()));
    }
}
