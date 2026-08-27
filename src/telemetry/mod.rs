//! Built-in OpenTelemetry integration (ADR 0068, spec 017).
//!
//! Every command dispatch is automatically wrapped in a `cli.command` span
//! carrying the command path and [invocation surface]. Handler authors can also
//! emit their own signals through [`AppContext::telemetry`], which returns a
//! [`Telemetry`] handle.
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
//! # Current limitations
//!
//! - **No context propagation.** No `traceparent` header is injected or
//!   extracted, so a trace does not yet span process boundaries (spec 017
//!   R23/R24). Each service produces its own disconnected trace.
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

pub use config::TelemetryConfig;
pub use guard::TelemetryGuard;
pub use handle::{Counter, Histogram, SpanHandle, Telemetry};
pub use noop::NoopTelemetry;
