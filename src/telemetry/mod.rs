//! Built-in OpenTelemetry integration (ADR 0068, spec 017).
//!
//! Every command dispatch is automatically wrapped in a `cli.command` span
//! carrying the command path and [invocation surface], and every HTTP request
//! served by `ApiServer` in an `http.request` server span carrying the matched
//! route, method and status. Handler authors can also emit their own signals
//! through [`AppContext::telemetry`], which returns a [`Telemetry`] handle.
//!
//! An application does **not** need to add request instrumentation of its own,
//! and should not: its span would nest inside the framework's rather than
//! replace it. To attach app-specific detail (tenant, principal, product),
//! `tracing::info!` inside the handler — events land on the enclosing
//! `http.request` span.
//!
//! The whole subsystem is opt-in behind the `telemetry` cargo feature. When the
//! feature is off — or on but no OTLP endpoint is configured — every call
//! resolves to [`NoopTelemetry`] and costs nothing. Enable export by handing a
//! [`TelemetryConfig`] to [`AppBuilder::with_telemetry`] or
//! [`ApiServerBuilder::with_telemetry`]; both export via an async
//! `BatchSpanProcessor`, flushed by [`TelemetryGuard`] on drop.
//!
//! # The subscriber
//!
//! `tracing` spans only become OTel spans if a `tracing-opentelemetry` layer is
//! installed into the **active subscriber**. `with_telemetry()` therefore
//! installs a process-wide subscriber (env-filter + `fmt` to stderr + the OTel
//! bridge). If the application has already installed its own global subscriber,
//! that call cannot take effect — the framework prints a warning to stderr and
//! exports nothing. Compose [`init::otel_layer`] into your own subscriber in
//! that case.
//!
//! # Distributed tracing
//!
//! Inbound `traceparent`/`tracestate` headers are extracted automatically and
//! become the parent of the `http.request` span, so a request arriving from
//! another instrumented service continues that service's trace instead of
//! starting a new one. Outbound calls are **not** automatic — the framework does
//! not own your HTTP client — so propagate them explicitly:
//!
//! ```ignore
//! use cli_framework::telemetry::propagation::TracedRequestBuilder as _;
//! let resp = client.get(url).with_trace_context().send().await?;
//! ```
//!
//! See [`propagation`] for the full contract, including why baggage is
//! deliberately not propagated.
//!
//! # Current limitations
//!
//! - **`http/protobuf` only.** It is the sole protocol this crate can export
//!   with. `OTEL_EXPORTER_OTLP_PROTOCOL` set to anything else (e.g. `grpc`) is
//!   now **rejected at init** with a message on stderr, and telemetry stays off
//!   rather than being exported over a protocol you did not ask for (spec 017
//!   R19). gRPC would need the `grpc-tonic` feature.
//! - **[`SpanHandle::set_attr`]** can only record fields that were declared at
//!   the span's callsite (`tracing`'s fieldset is fixed per callsite), so
//!   arbitrary keys are dropped. [`SpanHandle::record_error`] works because its
//!   `otel.status_*` fields are pre-declared.
//! - `traces_enabled` and `metrics_enabled` are honoured. `logs_enabled` is
//!   reserved — there is no OTLP logs pipeline yet, so it is reader-visible
//!   intent only (spec 020 item 5). `record_arg_values` and
//!   `arg_value_allowlist` are likewise reserved.
//!
//! # Authenticating to the collector
//!
//! Set [`TelemetryConfig::headers`], or the standard `OTEL_EXPORTER_OTLP_HEADERS`
//! environment variable, and they are sent with every OTLP request for **both**
//! traces and metrics. Values may be percent-encoded, which is how one
//! containing `,` or `=` survives the list format.
//!
//! These are credentials, so `TelemetryConfig`'s `Debug` impl prints header
//! names and **redacts every value** — a config reaching a log cannot leak a
//! bearer token.
//!
//! [invocation surface]: crate::app::dispatch::InvocationSurface
//! [`AppContext::telemetry`]: crate::app::AppContext::telemetry
//! [`AppBuilder::with_telemetry`]: crate::app::AppBuilder::with_telemetry

pub mod config;
pub mod guard;
pub mod handle;
#[cfg(feature = "telemetry")]
pub mod init;
pub mod noop;
#[cfg(feature = "telemetry")]
pub mod propagation;

pub use config::TelemetryConfig;
pub use guard::TelemetryGuard;
pub use handle::{Counter, Histogram, SpanHandle, Telemetry};
pub use noop::NoopTelemetry;

pub mod axes;
pub use axes::{Attribution, Deployment, ParseAxisError, TelemetryLevel};

pub mod probe;
pub use probe::{
    feature_outcome, FeatureOutcome, ProbeIdError, ProbeRegistry, ProbeSpec, OWNED_PROBE_LEAF,
};

// Gated on `telemetry`, unlike `axes`/`probe` above: this module hard-depends
// on `crate::config::resolution::Layer` (the top-level `config` module,
// itself behind `#[cfg(feature = "config")]`), which only exists when the
// `telemetry` feature's widened definition pulls `config` in. Left
// unconditional, a default build (no `telemetry`, no `config`) fails to
// compile the whole crate — the same reason `init`/`propagation` above are
// gated rather than left bare.
#[cfg(feature = "telemetry")]
pub mod policy;
#[cfg(feature = "telemetry")]
pub use policy::{
    detect_kill_switch, env_var_prefix, resolve_policy, KillSwitch, LayeredLevel, TelemetryInputs,
    TelemetryPolicy,
};

// Gated on `telemetry` for the same reason as `policy` above: `store.rs`
// hard-depends on `crate::config::{ConfigStore, ConfigFormat, FileBackend,
// VersionedConfig}`, which only exists when `config` is enabled. Left
// unconditional, a default build fails the same way the un-gated plan
// snippet for `policy` did.
#[cfg(feature = "telemetry")]
pub mod store;
#[cfg(feature = "telemetry")]
pub use store::{StoreState, TelemetrySettings, TelemetryStore, TELEMETRY_SCHEMA_VERSION};

// Gated on `observability`, not `telemetry`: `install_default_logging`/
// `LoggingGuard` replace `init_default_logging`'s old body and must stay
// reachable under `observability` alone, exactly as `init_default_logging`
// was before this module existed. The subscriber-composition items that do
// need the OTel bridge (`SubscriberOutcome`, `install_telemetry_subscriber`,
// `foreign_subscriber_finding`, ...) are individually gated on `telemetry`
// inside `subscriber.rs` and re-exported under that stronger gate below.
#[cfg(feature = "observability")]
pub mod subscriber;
#[cfg(feature = "telemetry")]
pub use subscriber::{
    foreign_subscriber_finding, install_subscriber_for_test, install_telemetry_subscriber,
    warn_once_foreign_subscriber, BoxedLayer, SubscriberOutcome,
};
#[cfg(feature = "observability")]
pub use subscriber::{install_default_logging, LoggingGuard};

// The `cli.panic` probe. Self-contained (only `std::panic`), but scoped to
// `telemetry` like `policy`/`store` above: it is one of this PR's telemetry
// probes, not a general-purpose logging utility, and its test target
// (`unit_telemetry_panic`) is `required-features = ["telemetry"]` too.
#[cfg(feature = "telemetry")]
pub mod panic;
#[cfg(feature = "telemetry")]
pub use panic::{install_panic_hook, panic_record, PanicRecord};

// Gated on `telemetry`, like `policy`/`store`/`panic` above: `startup.rs`
// names `KillSwitch`, `StoreState` and `SubscriberOutcome`, all of which only
// exist under this same feature. It only pins the fixed startup order and
// the `StartupReport` shape — the wiring that actually walks the order lands
// in PR7.
#[cfg(feature = "telemetry")]
pub mod startup;
#[cfg(feature = "telemetry")]
pub use startup::{startup_order, StartupReport, StartupStep};

// Gated on `telemetry` for the same reason as `policy`/`store` above:
// `manifest.rs` hard-depends on `crate::config::manifest::{ConfigManifest,
// FieldKind, FieldManifest, Scope}`, which only exists when `config` is
// enabled.
#[cfg(feature = "telemetry")]
pub mod manifest;
#[cfg(feature = "telemetry")]
pub use manifest::{
    merge_telemetry_section, telemetry_only_manifest, telemetry_section, ManifestMergeError,
    TELEMETRY_SECTION_KEY,
};

// Gated on `telemetry` for the same reason as `manifest` above: `env.rs`
// hard-depends on `crate::config::manifest::{ConfigManifest, FieldKind}`,
// which only exists when `config` is enabled.
#[cfg(feature = "telemetry")]
pub mod env;
#[cfg(feature = "telemetry")]
pub use env::{env_var_name, scan_environment, EnvScan};
// Gated on `telemetry` for the same reason as `policy`/`store` above:
// `resource.rs` takes `&TelemetryPolicy` and builds `opentelemetry_sdk::Resource`.
#[cfg(feature = "telemetry")]
pub mod resource;
#[cfg(feature = "telemetry")]
pub use resource::{
    apply_env_resource_attributes, metric_resource_attrs, to_resource, trace_resource_attrs,
    ServiceIdentity,
};
