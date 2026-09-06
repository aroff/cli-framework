// src/telemetry/author.rs
//! What an application author declares about telemetry at build time.
//!
//! These are the three values [`AppBuilder`][crate::app::AppBuilder] carries
//! from the author into [`resolve_policy`][super::resolve_policy]: the
//! transport defaults an operator may override, the identity the app can
//! attach once it knows who is calling, and the shapes of the two attribute
//! lists. They live here rather than on the builder because they are a
//! *declaration*, not builder state: the notice, the redaction rules and the
//! exporter all read them, and none of those should have to name
//! `crate::app`.
//!
//! Nothing here is a decision. `resolve_policy` makes every decision; this
//! module only carries the author's half of the inputs.

use std::sync::Arc;

/// The transport defaults an author picks, all three overridable by whoever
/// runs the process.
///
/// `endpoint` and `headers` are deliberately *defaults*, not settings: the
/// author knows where their own collector usually lives, but the operator
/// running the binary decides where this process actually sends. Both lose to
/// the corresponding `OTEL_EXPORTER_OTLP_*` variable.
///
/// `headers` is a [`SecretString`][secrecy::SecretString] and stays off the
/// config tree entirely — a manifest field is readable, roamable and
/// printable, and this one is usually a bearer token. Its `Debug` renders as
/// `Secret([REDACTED …])`, so a `TelemetryDefaults` in a log line leaks
/// nothing.
#[derive(Debug, Clone, Default)]
pub struct TelemetryDefaults {
    /// Where this app usually sends. OTLP `http/protobuf` only.
    /// `OTEL_EXPORTER_OTLP_ENDPOINT` wins over it.
    pub endpoint: Option<String>,
    /// OTLP headers, off the config tree.
    /// `OTEL_EXPORTER_OTLP_HEADERS` wins over it.
    pub headers: Option<secrecy::SecretString>,
    /// Argument names whose *values* may be recorded at `debug`, under the
    /// `cli.command.arg_values` probe. Every other argument records its name
    /// only.
    pub arg_value_allowlist: Vec<String>,
}

/// Who the caller is, when the app knows and the attribution axis permits it.
///
/// Both fields are optional because an app may know a tenant without knowing
/// a person, or the reverse, and neither is ever inferred.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Identity {
    /// The `enduser.id` attribute. Stable per person within the app's own
    /// namespace; never an email address or a login name.
    pub enduser_id: Option<String>,
    /// The tenant or organisation the caller acts for.
    pub tenant: Option<String>,
}

/// How an app answers "who is calling?".
///
/// A closure rather than a value because identity is not known at build time:
/// a CLI resolves it after authentication, and a service resolves it per
/// request. The framework calls this when it needs to attach identity and
/// the resolved [`Attribution`][super::Attribution] allows it, so an app that
/// never authenticates simply returns `None` and pays nothing.
pub type IdentityResolver =
    Arc<dyn Fn(&dyn crate::app::AppContext) -> Option<Identity> + Send + Sync>;
