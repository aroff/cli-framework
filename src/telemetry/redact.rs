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
//!    it. [`NEVER_KEYS`] adds the whole keys the specification forbids at any
//!    level — raw URLs, host name, command line — and [`NEVER_LIST_EXEMPT`]
//!    carves out the one framework key the substring rule would otherwise make
//!    unreachable.
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

/// Keys spec 025 forbids "at any level", independently of the level table.
///
/// The specification's never-at-any-level list (§"Attribute rules") names raw
/// URLs and paths, host names and command lines. The redacting exporter is
/// "the only place these rules live", so the boundary enforces them here
/// rather than trusting every present and future instrumentation site never to
/// have set them. `url.*` is on the list because a raw URL carries a path and
/// a query string; `http.route` — the matched template, which is bounded and
/// carries no user data — is the alternative and is unaffected.
///
/// Exact matches, lower-cased. Unlike [`NEVER_LIST`] these are whole keys, not
/// fragments: `url.` as a fragment would also catch a hypothetical
/// `cli.url.scheme`, and guessing at keys that do not exist is how a
/// never-list starts dropping things nobody intended.
pub const NEVER_KEYS: &[&str] = &[
    "url.full",
    "url.path",
    "url.query",
    "host.name",
    "process.command_line",
];

/// Keys this crate declares and reviews itself, exempt from [`NEVER_LIST`]'s
/// substring match.
///
/// The never-list is a heuristic over *unknown* key names: a key called
/// `access_token` is almost certainly a credential, so `*token*` is the right
/// default. But `cli.usage_error.token` is a parse token — the offending word
/// on a mistyped command line — and spec 025 requires it on the usage-error
/// event at `debug`, then says the boundary "clears `exception.message` and
/// `cli.usage_error.token` below `debug`", which is only meaningful if it
/// keeps them *at* `debug`. Without this exemption the substring rule makes
/// that probe permanently unreachable and its [`ELEVATED`] entry dead code.
///
/// Exact matches only, and only against the built-in list: an author who
/// extends the never-list with `token` still drops this key, because extending
/// it is a deliberate act by someone who has decided their product cannot
/// carry the value.
pub const NEVER_LIST_EXEMPT: &[&str] = &[
    "cli.usage_error.token",
    // `secrets.backend` is a backend *name* (`keychain`, `file`) and
    // `secrets.op` is one of `get`, `set`, `delete`. Neither is a secret, a
    // secret's name, or a secret's value — spec 025 row `cli.secrets` asks
    // for exactly these two keys and then says "never names or values",
    // which is the distinction. The substring rule fires on the probe
    // family's own name, so without this carve-out the whole `cli.secrets`
    // probe is catalogued, instrumented, and permanently unreachable.
    "secrets.backend",
    "secrets.op",
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
    // Bare namespaces spec 025 puts on framework spans: `process.exit.code`
    // on the root span (row `cli.process`), `config.schema_version` /
    // `config.backend` / `config.policy.state` on the `cli.config.*` child
    // spans (row `cli.config`), and `secrets.backend` / `secrets.op` on
    // `cli.secrets.op` (row `cli.secrets`). They carry no `cli.` prefix
    // because the specification wrote them without one, so rule 3 would
    // otherwise treat the framework's own normative attributes as
    // application attributes and require every app to allowlist them.
    //
    // Widening a prefix widens it for applications too: an app key called
    // `config.tier` now reaches the boundary without `with_telemetry_attrs`.
    // That is the same trade already made for `cli.`, `http.` and `server.`,
    // and it is bounded by the never-list, which runs first and is
    // unaffected — `process.command_line` stays dropped by `NEVER_KEYS`
    // regardless of this list.
    "process.",
    "config.",
    "secrets.",
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
    // `cli.command.args` is the probe *id*; the two entries below it are the
    // attribute *keys* the root span records for that probe
    // (`src/app/builder.rs`). `attribute_min_level` matches keys exactly, so
    // listing only the id here protects nothing: without these two lines an
    // install at `usage` exports the names of every argument that was
    // supplied, which PRD 025 reserves for `diagnostic`. The span records
    // them unconditionally by design -- redaction is enforced at the export
    // boundary and nowhere else -- so this table is the only thing that
    // holds them back.
    ("cli.command.arg_names", TelemetryLevel::Diagnostic),
    ("cli.command.arg_count", TelemetryLevel::Diagnostic),
    ("cli.command.args", TelemetryLevel::Diagnostic),
    ("server.address", TelemetryLevel::Diagnostic),
    ("http.client.server_address", TelemetryLevel::Diagnostic),
];

/// Is this key on the never-list, built-in or author-extended?
pub fn is_never_listed(key: &str, extra: &[String]) -> bool {
    let lowered = key.to_ascii_lowercase();

    // Whole keys the specification forbids outright. First, because nothing
    // below — not the exemption, not a level — may reach past them.
    if NEVER_KEYS.contains(&lowered.as_str()) {
        return true;
    }

    // The author's own additions, before the exemption: extending the
    // never-list is a deliberate decision about this product, and it outranks
    // this crate's judgement about its own keys.
    if extra
        .iter()
        // An empty fragment makes `contains` true for every key, so a single
        // stray `""` in `with_telemetry_never` would silently strip every
        // attribute in the product — telemetry that looks alive and carries
        // nothing. Ignore blanks instead.
        .filter(|f| !f.trim().is_empty())
        .any(|f| lowered.contains(&f.to_ascii_lowercase()))
    {
        return true;
    }

    if NEVER_LIST_EXEMPT.contains(&key) {
        return false;
    }

    NEVER_LIST.iter().any(|f| lowered.contains(f))
}

/// The lowest telemetry level at which this key may be recorded.
pub fn attribute_min_level(key: &str) -> TelemetryLevel {
    // `cli.command.arg_values.<name>` is a dynamic key — one per allowlisted
    // argument name — so it cannot appear in `ELEVATED` by exact match the
    // way `cli.usage_error.token` can. Match the prefix instead, before the
    // table lookup, so every such key inherits its parent probe's debug-only
    // minimum regardless of which argument name follows the dot.
    if key.starts_with("cli.command.arg_values.") {
        return TelemetryLevel::Debug;
    }
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
