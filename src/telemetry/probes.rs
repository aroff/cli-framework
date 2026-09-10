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

    /// Every instrument. Its emission status — wired, or reserved and why —
    /// is in [`METRIC_EMISSION`](super::METRIC_EMISSION), which
    /// `tests/unit/telemetry_probe_census.rs` checks against the actual call
    /// sites in `src/` in both directions.
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

/// Span names, split into [`ROOTS`](spans::ROOTS) and
/// [`CHILDREN`](spans::CHILDREN): by the spec's rule a usage-level probe adds
/// to a root span and only a diagnostic-level probe opens a child, so which
/// list a name is in is a statement about the level it becomes visible at.
///
/// Emission status is in [`SPAN_EMISSION`].
pub mod spans {
    pub const ROOT_COMMAND: &str = "cli.command";
    pub const CONFIG_LOAD: &str = "cli.config.load";
    pub const CONFIG_MIGRATE: &str = "cli.config.migrate";
    pub const CONFIG_POLICY_REFRESH: &str = "cli.config.policy_refresh";
    pub const SECRETS_OP: &str = "cli.secrets.op";
    pub const PLUGIN_LOAD: &str = "cli.plugin.load";
    pub const HTTP_CLIENT_REQUEST: &str = "http.client.request";
    pub const HTTP_SERVER_REQUEST: &str = "http.request";

    /// The child spans only.
    ///
    /// Both root spans are deliberately absent. `ROOT_COMMAND` is the CLI's
    /// own root, and `HTTP_SERVER_REQUEST` is the API surface's: spec 025 §4
    /// calls it "the existing root span `http.request`", and the axum
    /// middleware in `src/api/mod.rs` opens it per request, continuing the
    /// caller's trace when one was propagated rather than nesting under a
    /// command. It was listed here originally, which forced
    /// `only_diagnostic_probes_own_child_spans` to skip it by name — a test
    /// carrying a `continue` for the one entry that breaks its rule is the
    /// signal the entry is in the wrong list, not that the rule needs an
    /// exception.
    pub const CHILDREN: &[&str] = &[
        CONFIG_LOAD,
        CONFIG_MIGRATE,
        CONFIG_POLICY_REFRESH,
        SECRETS_OP,
        PLUGIN_LOAD,
        HTTP_CLIENT_REQUEST,
    ];

    /// The spans that start a trace: the CLI's own root and the API
    /// surface's. `CHILDREN` and `ROOTS` together are the whole span catalog,
    /// which is what lets the census in
    /// `tests/unit/telemetry_probe_census.rs` assert coverage of both.
    pub const ROOTS: &[&str] = &[ROOT_COMMAND, HTTP_SERVER_REQUEST];
}

/// Whether a catalogued name is emitted by production code today.
///
/// # Why this table exists
///
/// The probe catalog is a *normative* list: spec 025 §4 defines it, and its
/// `sends` column is the disclosure contract `telemetry info` renders to end
/// users. A catalogued name with no call site is therefore not merely dead
/// code — it is a promise the binary does not keep, and
/// `specs/028-telemetry-probe-emission-gap.md` records that **no ordinary
/// gate can detect one**: the attribute builders are `pub` and re-exported, so
/// rustc reports no dead code and clippy is silent, and `probes.rs` measured
/// 100% line coverage while eleven of its builders had no caller, because the
/// unit tests call every builder directly. Coverage cannot detect an unused
/// public API.
///
/// This table is the missing gate. Every name in [`metrics::ALL`],
/// [`spans::CHILDREN`] and [`spans::ROOTS`] appears here exactly once, and
/// `tests/unit/telemetry_probe_census.rs` checks the claim each entry makes
/// against the actual text of `src/` — in *both* directions. A `Wired` entry
/// whose call site is deleted fails the census; a `Reserved` entry that
/// someone wires without updating this table also fails it. Neither can drift
/// silently, which is the property the catalog lacked.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Emission {
    /// Production code creates this instrument or opens this span. The
    /// payload is the file the call site lives in, relative to the crate
    /// root, so the census can check the site is where the table says.
    Wired(&'static str),
    /// Catalogued, deliberately not emitted yet. The payload is why.
    ///
    /// `Reserved` is a statement about *this* crate's call sites, not about
    /// the name's validity: the name stays in the catalog, `telemetry info`
    /// keeps disclosing the probe that owns it, and the attribute builder
    /// stays public and tested. Deleting the entry instead would shrink the
    /// disclosure contract below what spec 025 requires, which is a worse
    /// outcome than an honest gap.
    Reserved(&'static str),
}

impl Emission {
    pub fn is_wired(&self) -> bool {
        matches!(self, Self::Wired(_))
    }
}

/// The reason that gates *every* `Reserved` entry below, in addition to
/// whatever local obstacle each one names.
///
/// This const previously read `RESERVED_PENDING_REDACTION`, and named a
/// different obstacle: that
/// [`init_from_policy`](crate::telemetry::init_from_policy) — the only
/// constructor of either half of the export boundary — had no production
/// caller, so `App::run_with_args` and `ApiServer::serve` both exported
/// through a bare `span_exporter()` and a meter provider with no View.
/// While that held, wiring a probe was not merely an unfilled gap: it pushed
/// `cli.probe`, the install id and diagnostic-level attributes onto the wire
/// unstripped, at a level the person running the CLI never consented to.
/// Spec 028 called the ordering a safety property for exactly that reason.
///
/// That obstacle is gone. `run_startup` (`src/telemetry/startup.rs`) calls
/// `init_from_policy`, so the redacting span exporter and the per-instrument
/// Views are on the production startup path, and
/// `the_redacting_export_boundary_is_on_the_production_startup_path` in
/// `tests/unit/telemetry_probe_census.rs` fails if that regresses. What gates
/// the remaining reservations is now ordinary unfinished work — no call site
/// has been written — which is what spec 028's "Scope of the work" tracks.
/// The distinction matters: adding an emitter is now a safe change to make,
/// where before it was not.
pub const RESERVED_PENDING_INSTRUMENTATION: &str =
    "the export boundary has landed (run_startup calls init_from_policy), so what remains is \
     writing the call site; see spec 028's Scope of the work";

/// Emission status of every instrument in [`metrics::ALL`].
pub const METRIC_EMISSION: &[(&str, Emission)] = &[
    (
        metrics::COMMAND_INVOCATIONS,
        Emission::Wired("src/app/builder.rs"),
    ),
    (
        metrics::COMMAND_DURATION_MS,
        Emission::Wired("src/app/builder.rs"),
    ),
    (
        metrics::PROCESS_DURATION_MS,
        Emission::Reserved(
            "the interval is startup-to-flush, which only the startup wiring can measure; that \
             wiring is plan Task 27 and is itself what would replace init_batch with \
             init_from_policy",
        ),
    ),
    (
        metrics::USAGE_ERRORS,
        Emission::Reserved(
            "dispatch classifies usage errors for the exit code but reaches no Telemetry \
             handle at the point it does so",
        ),
    ),
    (metrics::PANICS, Emission::Wired("src/telemetry/startup.rs")),
    (
        metrics::HELP_SHOWN,
        Emission::Reserved("the help renderer carries no Telemetry handle"),
    ),
    (
        metrics::FEATURE_USES,
        Emission::Reserved(
            "the span side is wired (feature_attrs, src/app/context.rs); the counter \
             additionally needs the cli.feature.<name> registration check spec 025 puts in \
             front of it",
        ),
    ),
    (
        metrics::AUTH_EVENTS,
        Emission::Reserved("the auth/token providers carry no Telemetry handle"),
    ),
    (
        metrics::DOCTOR_FINDINGS,
        Emission::Reserved(
            "doctor runs its checks before any telemetry handle is threaded into the doctor \
             module",
        ),
    ),
    (
        metrics::PLUGIN_LOADS,
        Emission::Reserved("the plugin registry carries no Telemetry handle"),
    ),
    (
        metrics::CHAT_TURNS,
        Emission::Reserved("the chat loop carries no Telemetry handle"),
    ),
    (
        metrics::CHAT_SESSIONS,
        Emission::Reserved("the chat loop carries no Telemetry handle"),
    ),
    (
        metrics::HTTP_CLIENT_REQUEST_DURATION,
        Emission::Reserved(
            "RetryableHttpClient carries no AppContext or Telemetry handle at the call site, \
             so http.client is span-only today — see the comment above the span in \
             src/http_retry/client.rs",
        ),
    ),
    (
        metrics::HTTP_SERVER_REQUEST_DURATION,
        Emission::Reserved(
            "the axum middleware in src/api/mod.rs closes over the request alone and reaches \
             no Telemetry handle, the same obstacle as http.client",
        ),
    ),
    (metrics::MCP_TOOL_CALLS, Emission::Wired("src/mcp/mod.rs")),
];

/// Emission status of every span in [`spans::ROOTS`] and [`spans::CHILDREN`].
pub const SPAN_EMISSION: &[(&str, Emission)] = &[
    (spans::ROOT_COMMAND, Emission::Wired("src/app/builder.rs")),
    (
        spans::HTTP_SERVER_REQUEST,
        Emission::Wired("src/api/mod.rs"),
    ),
    (
        spans::HTTP_CLIENT_REQUEST,
        Emission::Wired("src/http_retry/client.rs"),
    ),
    (
        spans::CONFIG_LOAD,
        Emission::Reserved(
            "the config backends load without a Telemetry handle in scope; spec 025 also wants \
             config.schema_version, config.backend and config.policy.state on the span, and \
             policy.state has no source until the policy client exists",
        ),
    ),
    (
        spans::CONFIG_MIGRATE,
        Emission::Reserved("config migration runs inside the backends, with no handle in scope"),
    ),
    (
        spans::CONFIG_POLICY_REFRESH,
        Emission::Reserved(
            "there is no policy client to refresh from yet — the same gap \
             specs/029-telemetry-deferred-wiring.md item 3 records",
        ),
    ),
    (
        spans::SECRETS_OP,
        Emission::Reserved(
            "the secret-store backends carry no Telemetry handle. Note the attributes are \
             ready: secrets.backend and secrets.op are already carved out of the never-list \
             (NEVER_LIST_EXEMPT) precisely so this span can carry them",
        ),
    ),
    (
        spans::PLUGIN_LOAD,
        Emission::Reserved("the plugin registry carries no Telemetry handle"),
    ),
];

/// Emission status of every probe-data builder in this file.
///
/// # Why attributes need their own table
///
/// Spec 025 §4 promises *attributes*, not only spans and metrics: the
/// `cli.process` row promises the root-span attribute `process.exit.code`, and
/// most other rows promise a set of keys rather than an instrument. Those keys
/// exist in exactly one place each — the builder below that returns them — so
/// a builder with no caller is a promise the binary does not keep, in the same
/// way an uncreated instrument is. Censusing only [`metrics::ALL`] and the
/// span catalog would have answered the narrow question and left the larger
/// one open: eleven of the eighteen builders here have no production caller.
///
/// `tests/unit/telemetry_probe_census.rs` derives the set of builders from
/// this file's own signatures rather than from a second hand-written list, so
/// a builder added without an entry here fails the census instead of joining
/// the catalog uncensused. The rule it derives them by is the return type — a
/// `pub fn` here yielding a `Vec` produces probe data — which is why
/// [`registered_feature_names`] is in the table beside the `_attrs` builders
/// even though what it returns is names rather than `KeyValue`s. It is also
/// the entry that proves the census counts a builder *reached* rather than a
/// builder *called*: `src/app/context.rs` passes it to `.map(...)` as a
/// function value, writing no parentheses of its own.
pub const BUILDER_EMISSION: &[(&str, Emission)] = &[
    (
        "command_metric_labels",
        Emission::Wired("src/app/builder.rs"),
    ),
    ("command_span_attrs", Emission::Wired("src/app/builder.rs")),
    ("feature_attrs", Emission::Wired("src/app/context.rs")),
    (
        "http_client_attrs",
        Emission::Wired("src/http_retry/client.rs"),
    ),
    ("http_server_attrs", Emission::Wired("src/api/mod.rs")),
    ("mcp_session_attrs", Emission::Wired("src/mcp/mod.rs")),
    (
        "registered_feature_names",
        Emission::Wired("src/app/context.rs"),
    ),
    (
        "arg_names",
        Emission::Reserved(
            "bypassed rather than blocked: the root span in src/app/builder.rs joins the \
             argument names inline. The body is names.to_vec() today, so routing through it \
             would change nothing observable; it is the seam the cli.command.args policy needs \
             once there is filtering to apply, and until then wiring it would only look like \
             progress",
        ),
    ),
    (
        "arg_value_attrs",
        Emission::Reserved(
            "cli.command.arg_values is debug-level and keyed by an author-supplied allowlist; \
             nothing reads that allowlist onto the root span yet, and this is the one builder \
             whose output is argument values rather than names",
        ),
    ),
    (
        "usage_error_attrs",
        Emission::Reserved(
            "dispatch classifies the usage error for the exit code, but DiagnosticReporter \
             prints it and App::run exits 2 without ever touching the open root span — the \
             same obstacle that reserves the cli.usage_errors metric",
        ),
    ),
    (
        "auth_attrs",
        Emission::Reserved("the auth/token providers carry no Telemetry handle"),
    ),
    (
        "chat_attrs",
        Emission::Reserved("the chat loop carries no Telemetry handle"),
    ),
    (
        "config_attrs",
        Emission::Reserved(
            "produced for the three cli.config.* child spans, all of which are Reserved for \
             the same reason: the config backends load with no handle in scope",
        ),
    ),
    (
        "doctor_attrs",
        Emission::Reserved(
            "doctor runs its checks before any telemetry handle is threaded into the doctor \
             module",
        ),
    ),
    (
        "help_attrs",
        Emission::Reserved("the help renderer carries no Telemetry handle"),
    ),
    (
        "plugin_attrs",
        Emission::Reserved("the plugin registry carries no Telemetry handle"),
    ),
    (
        "process_attrs",
        Emission::Reserved(
            "the framework never learns the process exit status: App::run_with_args returns \
             Result<()> and the application's own main maps it to a code. The one status the \
             framework picks itself, std::process::exit(2) in App::run, is reached after \
             run_with_args has returned — the root span is closed and the telemetry guard \
             dropped — and process::exit skips destructors, so nothing could flush a late \
             record. process.exit.code needs the process-lifetime scope that \
             cli.process.duration_ms is also waiting on, not merely a caller for this builder",
        ),
    ),
    (
        "secrets_attrs",
        Emission::Reserved(
            "the secret-store backends carry no Telemetry handle, the same obstacle that \
             reserves the cli.secrets.op span these attributes belong to",
        ),
    ),
];

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
/// `server.address` (+ `server.port`) is the PRD's separate `http.client.server_address`
/// diagnostic probe, span-only, so an operator debugging a connection can see
/// the host and port without either becoming a metric label.
pub fn http_client_attrs(
    method: &str,
    status: Option<u16>,
    server_address: Option<&str>,
    server_port: Option<u16>,
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
        attrs.push(KeyValue::new("server.address", address.to_string()));
    }
    if let Some(port) = server_port {
        attrs.push(KeyValue::new("server.port", port.to_string()));
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
/// `mcp.tool` comes from the server's own declared tool list — bounded the
/// same way a registered command path is. `status` is not known when the
/// span is created (the call has not run yet): callers pass `None` at
/// creation and call this again with `Some(status)` once the outcome is
/// known, recording only the newly-known field onto the still-open span.
pub fn mcp_session_attrs(tool: Option<&str>, status: Option<&str>) -> Vec<KeyValue> {
    let mut attrs = vec![KeyValue::new(PROBE_ATTR_KEY, "mcp.session")];
    if let Some(tool) = tool {
        attrs.push(KeyValue::new("mcp.tool", tool.to_string()));
    }
    if let Some(status) = status {
        attrs.push(KeyValue::new("status", status.to_string()));
    }
    attrs
}

/// Attributes for the `cli.chat` probe.
///
/// There is no attribute here under which prompt or reply text could travel,
/// at any telemetry level including debug: only the turn count and the
/// session's duration. Debug is a troubleshooting level, not a bypass, and a
/// prompt is the single highest-value piece of personal data a CLI touches.
pub fn chat_attrs(turn_count: u64, duration_ms: f64) -> Vec<KeyValue> {
    vec![
        KeyValue::new(PROBE_ATTR_KEY, "cli.chat"),
        KeyValue::new("cli.chat.turn_count", turn_count.to_string()),
        KeyValue::new("cli.chat.duration_ms", duration_ms.to_string()),
    ]
}

/// Attributes for the `cli.doctor` probe.
///
/// `check` is the check's own id (a closed, author-defined vocabulary, like a
/// command path), `severity` its finding's severity — never the check's
/// free-text explanation. `findings_count` is a root-span attribute (the
/// total findings from the run), not a per-check metric label.
pub fn doctor_attrs(check: &str, severity: &str, findings_count: u64) -> Vec<KeyValue> {
    vec![
        KeyValue::new(PROBE_ATTR_KEY, "cli.doctor"),
        KeyValue::new("check", check.to_string()),
        KeyValue::new("severity", severity.to_string()),
        KeyValue::new("cli.doctor.findings_count", findings_count.to_string()),
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
/// Which operation ran (load, migrate, policy refresh) is already carried by
/// the child span's own name (see [`spans::CONFIG_LOAD`] and friends), so
/// unlike the other probes here there is no `kind`/operation attribute.
/// `schema_version` is the config schema's own version number, `backend`
/// names where it was read from (`file` or `registry`) and `policy_state`
/// whether an org policy governs it (`managed` or `not_managed`) — never the
/// setting's value.
pub fn config_attrs(schema_version: u32, backend: &str, policy_state: &str) -> Vec<KeyValue> {
    vec![
        KeyValue::new(PROBE_ATTR_KEY, "cli.config"),
        KeyValue::new("config.schema_version", schema_version.to_string()),
        KeyValue::new("config.backend", backend.to_string()),
        KeyValue::new("config.policy.state", policy_state.to_string()),
    ]
}

/// Attributes for the `cli.secrets` probe.
///
/// `backend` names which store handled the operation, `op` is a closed
/// vocabulary token (`get`, `set`, `delete`), and `status` is the outcome
/// (`ok`, `error`) — never the secret itself.
pub fn secrets_attrs(backend: &str, op: &str, status: &str) -> Vec<KeyValue> {
    vec![
        KeyValue::new(PROBE_ATTR_KEY, "cli.secrets"),
        KeyValue::new("secrets.backend", backend.to_string()),
        KeyValue::new("secrets.op", op.to_string()),
        KeyValue::new("status", status.to_string()),
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
///
/// `exit_code` is a root-span attribute: the process's own exit status,
/// never a value derived from command output.
pub fn process_attrs(exit_code: i32) -> Vec<KeyValue> {
    vec![
        KeyValue::new(PROBE_ATTR_KEY, "cli.process"),
        KeyValue::new("process.exit.code", exit_code.to_string()),
    ]
}
