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
//! - **No OTLP auth headers.** `OTEL_EXPORTER_OTLP_HEADERS` is not read, so a
//!   collector requiring authentication cannot be reached (spec 017 R25).
//! - **`http/protobuf` only.** `OTEL_EXPORTER_OTLP_PROTOCOL` is parsed onto the
//!   config but not acted on; the exporter is always HTTP (spec 017 R19).
//! - **[`SpanHandle::set_attr`]** can only record fields that were declared at
//!   the span's callsite (`tracing`'s fieldset is fixed per callsite), so
//!   arbitrary keys are dropped. [`SpanHandle::record_error`] works because its
//!   `otel.status_*` fields are pre-declared.
//! - Config fields `traces_enabled`, `logs_enabled`, `record_arg_values`, and
//!   `arg_value_allowlist` are reserved and not yet consulted. `metrics_enabled`
//!   *is* honoured.
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
