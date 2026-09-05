//! Attribute and label construction for the built-in probes.
//!
//! Every function here is pure: it takes a policy and an outcome and returns
//! attributes. The callsites in `src/app/builder.rs` and friends call these
//! and hand the result to the telemetry handle; the export boundary decides
//! what survives. Keeping the construction pure is what lets the whole probe
//! catalog be tested without a provider, a collector or a subscriber.

use super::policy::TelemetryPolicy;
use super::probe::{FeatureOutcome, ProbeRegistry};
use super::redact::{is_never_listed, PROBE_ATTR_KEY};
use crate::app::dispatch::InvocationSurface;
use opentelemetry::KeyValue;

/// Where an invocation came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Surface {
    Cli,
    Chat,
    Mcp,
    Api,
}

impl Surface {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Cli => "cli",
            Self::Chat => "chat",
            Self::Mcp => "mcp",
            Self::Api => "api",
        }
    }
}

/// Bridge from the dispatch layer's own surface enum.
///
/// `InvocationSurface` (`src/app/dispatch.rs`) and `Surface` are structurally
/// identical — same four variants, same string mapping — because they answer
/// the same question for two different audiences: `InvocationSurface` is
/// dispatch's internal routing tag, `Surface` is the probe catalog's public,
/// serializable attribute type. Keeping them as two types (rather than
/// reusing one, or deleting either) avoids coupling the dispatch module's
/// API to the telemetry probe catalog's; this conversion is the entire cost
/// of that separation.
impl From<InvocationSurface> for Surface {
    fn from(surface: InvocationSurface) -> Self {
        match surface {
            InvocationSurface::Cli => Self::Cli,
            InvocationSurface::Chat => Self::Chat,
            InvocationSurface::Mcp => Self::Mcp,
            InvocationSurface::Api => Self::Api,
        }
    }
}

/// How an invocation ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommandStatus {
    Ok,
    UsageError,
    Error,
}

impl CommandStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Ok => "ok",
            Self::UsageError => "usage_error",
            Self::Error => "error",
        }
    }
}

/// What the `cli.command` probe reports.
#[derive(Debug, Clone)]
pub struct CommandOutcome {
    /// `Some` only for a path the command registry actually declares.
    ///
    /// `None` for anything else — a typo, an unloaded plugin, an injected
    /// argument. An unvalidated string here would be both unbounded metric
    /// cardinality and a leak, because a mistyped command is frequently a
    /// mistyped path or a pasted credential.
    pub command: Option<String>,
    pub surface: Surface,
    pub status: CommandStatus,
    pub duration_ms: f64,
}

/// Labels for `cli.command.invocations` and `cli.command.duration_ms`.
pub fn command_metric_labels(outcome: &CommandOutcome) -> Vec<KeyValue> {
    let mut labels = Vec::with_capacity(3);
    if let Some(command) = &outcome.command {
        labels.push(KeyValue::new("command", command.clone()));
    }
    labels.push(KeyValue::new("surface", outcome.surface.as_str()));
    labels.push(KeyValue::new("status", outcome.status.as_str()));
    labels
}

/// Attributes for the root span.
pub fn command_span_attrs(policy: &TelemetryPolicy, outcome: &CommandOutcome) -> Vec<KeyValue> {
    let mut attrs = vec![KeyValue::new(PROBE_ATTR_KEY, "cli.command")];
    if let Some(id) = &policy.install_id {
        attrs.push(KeyValue::new("cli.install.id", id.clone()));
    }
    attrs.push(KeyValue::new("session.id", policy.session_id.clone()));
    attrs.push(KeyValue::new("cli.telemetry.level", policy.level.as_str()));
    attrs.extend(command_metric_labels(outcome));
    attrs
}

/// The names of the arguments a command received — never their values.
///
/// `--output` says a person used the output flag. `--output
/// /home/alice/tax-2025.csv` says their name and their tax situation.
pub fn arg_names(names: &[String]) -> Vec<String> {
    names.to_vec()
}

/// Values for the arguments the author allowlisted, as
/// `cli.command.arg_values.<name>` attributes.
///
/// The never-list is applied to the generated key, so an author who
/// allowlists `api_key` by mistake still gets nothing.
pub fn arg_value_attrs(allowlist: &[String], pairs: &[(String, String)]) -> Vec<KeyValue> {
    pairs
        .iter()
        .filter(|(name, _)| allowlist.iter().any(|a| a == name))
        .map(|(name, value)| (format!("cli.command.arg_values.{name}"), value.clone()))
        .filter(|(key, _)| !is_never_listed(key, &[]))
        .map(|(key, value)| KeyValue::new(key, value))
        .collect()
}

/// Attributes for `cli.usage_error` and its `cli.usage_error.token` child.
///
/// `kind` is a closed vocabulary token, never the error message: a message
/// carries the offending value inside it, which is exactly what must not
/// travel at usage level.
pub fn usage_error_attrs(kind: &str, token: Option<&str>) -> Vec<KeyValue> {
    let mut attrs = vec![
        KeyValue::new(PROBE_ATTR_KEY, "cli.usage_error"),
        KeyValue::new("cli.usage_error.kind", kind.to_string()),
    ];
    if let Some(token) = token {
        attrs.push(KeyValue::new("cli.usage_error.token", token.to_string()));
    }
    attrs
}

/// Adapt a [`ProbeRegistry`] into the plain name list [`super::feature_outcome`]
/// expects.
///
/// `feature_outcome` is PR1's pure function and takes `&[&str]`, not a
/// registry — reused here rather than widened or redefined. This is the seam
/// that lets a caller holding a `&ProbeRegistry` (as `AppContext::mark_feature`
/// does) reach it: every registered id of the form `cli.feature.<name>` has
/// its prefix stripped back off, in registry order (sorted by id, so this is
/// stable too).
pub fn registered_feature_names(registry: &ProbeRegistry) -> Vec<&str> {
    const PREFIX: &str = "cli.feature.";
    registry
        .iter()
        .filter_map(|spec| spec.id.strip_prefix(PREFIX))
        .collect()
}

/// Attributes for the `cli.feature` event.
///
/// The name always travels as a span attribute. It becomes a *metric label*
/// only when `outcome` is [`FeatureOutcome::Recorded`], because a label is a
/// permanent time series and an unregistered name is not necessarily
/// bounded — `mark_feature(&user_input)` in a loop would otherwise mint one
/// time series per input.
pub fn feature_attrs(name: &str, outcome: FeatureOutcome) -> Vec<KeyValue> {
    let mut attrs = vec![
        KeyValue::new(PROBE_ATTR_KEY, "cli.feature"),
        KeyValue::new("cli.feature.name", name.to_string()),
    ];
    if outcome == FeatureOutcome::Recorded {
        attrs.push(KeyValue::new("feature", name.to_string()));
    }
    attrs
}

// src/telemetry/probes.rs — the instrument and span catalog.
//
// Every name here is normative in spec 025 §4. Changing one is a breaking
// change for anybody's dashboard, so it changes here, once, with the test in
// Task 20 failing until the table and the spec agree again.

/// Metric instruments, by the probe that owns them.
pub mod metrics {
    pub const COMMAND_INVOCATIONS: &str = "cli.command.invocations";
    pub const COMMAND_DURATION_MS: &str = "cli.command.duration_ms";
    pub const PROCESS_DURATION_MS: &str = "cli.process.duration_ms";
    pub const USAGE_ERRORS: &str = "cli.usage_errors";
    pub const PANICS: &str = "cli.panics";
    pub const HELP_SHOWN: &str = "cli.help.shown";
    pub const FEATURE_USES: &str = "cli.feature.uses";
    pub const AUTH_EVENTS: &str = "cli.auth.events";
    pub const DOCTOR_FINDINGS: &str = "cli.doctor.findings";
    pub const PLUGIN_LOADS: &str = "cli.plugin.loads";
    pub const CHAT_TURNS: &str = "cli.chat.turns";
    pub const CHAT_SESSIONS: &str = "cli.chat.sessions";
    pub const HTTP_CLIENT_REQUEST_DURATION: &str = "http.client.request.duration";
    pub const HTTP_SERVER_REQUEST_DURATION: &str = "http.server.request.duration";
    pub const MCP_TOOL_CALLS: &str = "mcp.tool.calls";

    /// Every instrument, for the catalog test in Task 20.
    pub const ALL: &[&str] = &[
        COMMAND_INVOCATIONS,
        COMMAND_DURATION_MS,
        PROCESS_DURATION_MS,
        USAGE_ERRORS,
        PANICS,
        HELP_SHOWN,
        FEATURE_USES,
        AUTH_EVENTS,
        DOCTOR_FINDINGS,
        PLUGIN_LOADS,
        CHAT_TURNS,
        CHAT_SESSIONS,
        HTTP_CLIENT_REQUEST_DURATION,
        HTTP_SERVER_REQUEST_DURATION,
        MCP_TOOL_CALLS,
    ];
}

/// Span names. The root span is `cli.command`; everything else here is a
/// child span, which by the spec's rule means a diagnostic-level probe.
pub mod spans {
    pub const ROOT_COMMAND: &str = "cli.command";
    pub const CONFIG_LOAD: &str = "cli.config.load";
    pub const CONFIG_MIGRATE: &str = "cli.config.migrate";
    pub const CONFIG_POLICY_REFRESH: &str = "cli.config.policy_refresh";
    pub const SECRETS_OP: &str = "cli.secrets.op";
    pub const PLUGIN_LOAD: &str = "cli.plugin.load";
    pub const HTTP_CLIENT_REQUEST: &str = "http.client.request";
    pub const HTTP_SERVER_REQUEST: &str = "http.request";

    /// The child spans only — `ROOT_COMMAND` is deliberately absent.
    pub const CHILDREN: &[&str] = &[
        CONFIG_LOAD,
        CONFIG_MIGRATE,
        CONFIG_POLICY_REFRESH,
        SECRETS_OP,
        PLUGIN_LOAD,
        HTTP_CLIENT_REQUEST,
        HTTP_SERVER_REQUEST,
    ];
}

// The remaining fourteen probes (Task 20). Each function is pure, follows the
// shape Tasks 17-19 established — a `cli.probe` attribute first, then the
// probe's own keys, then `Option` fields pushed only when present — and is
// exercised without a provider, a collector or a subscriber in
// `tests/unit/telemetry_probe_surfaces.rs`.

/// Attributes for the `http.client` probe.
///
/// Deliberately no URL, in any form. A URL is the one place the never-list
/// cannot help: the secret is in the value, not in a key named `token`. Query
/// strings carry API keys, paths carry account identifiers, and both are
/// routinely pasted into a CLI by a person who has not thought about it.
/// `server_address` is a separate diagnostic probe, span-only, so an operator
/// debugging a connection can see the host without it becoming a metric label.
pub fn http_client_attrs(
    method: &str,
    status: Option<u16>,
    server_address: Option<&str>,
) -> Vec<KeyValue> {
    let mut attrs = vec![
        KeyValue::new(PROBE_ATTR_KEY, "http.client"),
        KeyValue::new("http.request.method", method.to_string()),
    ];
    if let Some(status) = status {
        attrs.push(KeyValue::new(
            "http.response.status_code",
            status.to_string(),
        ));
    }
    if let Some(address) = server_address {
        attrs.push(KeyValue::new(
            "http.client.server_address",
            address.to_string(),
        ));
    }
    attrs
}

/// Attributes for the `http.server` probe.
///
/// `route` must already be the matched template (`/v1/users/{id}`), never the
/// concrete path — this function trusts its caller on that point exactly as
/// `command_span_attrs` trusts `outcome.command` to already be a registered
/// path, because a route template is bounded (the application declares its
/// routes) while a concrete path is not.
pub fn http_server_attrs(route: &str, method: &str, status: u16) -> Vec<KeyValue> {
    vec![
        KeyValue::new(PROBE_ATTR_KEY, "http.server"),
        KeyValue::new("http.route", route.to_string()),
        KeyValue::new("http.request.method", method.to_string()),
        KeyValue::new("http.response.status_code", status.to_string()),
    ]
}

/// Attributes for the `mcp.session` probe.
///
/// `tool` comes from the server's own declared tool list — bounded the same
/// way a registered command path is — so unlike a free-text argument it is
/// safe as a metric label.
pub fn mcp_session_attrs(tool: Option<&str>) -> Vec<KeyValue> {
    let mut attrs = vec![KeyValue::new(PROBE_ATTR_KEY, "mcp.session")];
    if let Some(tool) = tool {
        attrs.push(KeyValue::new("tool", tool.to_string()));
    }
    attrs
}

/// Attributes for the `cli.chat` probe.
///
/// There is no attribute here under which prompt or reply text could travel,
/// at any telemetry level including debug: only the turn count. Debug is a
/// troubleshooting level, not a bypass, and a prompt is the single
/// highest-value piece of personal data a CLI touches.
pub fn chat_attrs(turns: u64) -> Vec<KeyValue> {
    vec![
        KeyValue::new(PROBE_ATTR_KEY, "cli.chat"),
        KeyValue::new("cli.chat.turns", turns.to_string()),
    ]
}

/// Attributes for the `cli.doctor` probe.
///
/// `check` is the check's own id (a closed, author-defined vocabulary, like a
/// command path), `severity` its finding's severity — never the check's
/// free-text explanation.
pub fn doctor_attrs(check: &str, severity: &str) -> Vec<KeyValue> {
    vec![
        KeyValue::new(PROBE_ATTR_KEY, "cli.doctor"),
        KeyValue::new("check", check.to_string()),
        KeyValue::new("severity", severity.to_string()),
    ]
}

/// Attributes for the `cli.plugin` probe.
///
/// `plugin` is the plugin's own declared name, `kind` a closed vocabulary
/// token (e.g. `load`, `unload`) — never a file path or an error message.
pub fn plugin_attrs(plugin: &str, kind: &str) -> Vec<KeyValue> {
    vec![
        KeyValue::new(PROBE_ATTR_KEY, "cli.plugin"),
        KeyValue::new("plugin", plugin.to_string()),
        KeyValue::new("kind", kind.to_string()),
    ]
}

/// Attributes for the `cli.auth` probe.
///
/// `kind` (login, logout, refresh) and `status` (ok, error) are both closed
/// vocabulary tokens — never a credential, a username or a token value.
pub fn auth_attrs(kind: &str, status: &str) -> Vec<KeyValue> {
    vec![
        KeyValue::new(PROBE_ATTR_KEY, "cli.auth"),
        KeyValue::new("kind", kind.to_string()),
        KeyValue::new("status", status.to_string()),
    ]
}

/// Attributes for the `cli.config` probe.
///
/// `kind` names which operation touched configuration (e.g. `get`, `set`,
/// `load`) — never the setting's value.
pub fn config_attrs(kind: &str) -> Vec<KeyValue> {
    vec![
        KeyValue::new(PROBE_ATTR_KEY, "cli.config"),
        KeyValue::new("kind", kind.to_string()),
    ]
}

/// Attributes for the `cli.secrets` probe.
///
/// `kind` names which backend or operation ran — never the secret itself.
pub fn secrets_attrs(kind: &str) -> Vec<KeyValue> {
    vec![
        KeyValue::new(PROBE_ATTR_KEY, "cli.secrets"),
        KeyValue::new("kind", kind.to_string()),
    ]
}

/// Attributes for the `cli.help` probe.
///
/// `command` is `Some` only for a path the command registry actually
/// declares, mirroring [`CommandOutcome::command`]'s same rule: an unbounded
/// string here would be both unbounded metric cardinality and a leak.
pub fn help_attrs(command: Option<&str>) -> Vec<KeyValue> {
    let mut attrs = vec![KeyValue::new(PROBE_ATTR_KEY, "cli.help")];
    if let Some(command) = command {
        attrs.push(KeyValue::new("command", command.to_string()));
    }
    attrs
}

/// Attributes for the `cli.process` probe.
pub fn process_attrs() -> Vec<KeyValue> {
    vec![KeyValue::new(PROBE_ATTR_KEY, "cli.process")]
}
