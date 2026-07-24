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
//! [`TelemetryConfig`] to [`AppBuilder::with_telemetry`] (CLI, exports via a
//! synchronous `SimpleSpanProcessor`) or [`ApiServerBuilder::with_telemetry`]
//! (long-running server, exports via an async `BatchSpanProcessor`).
//!
//! # Current limitations (v1)
//!
//! - **Traces only.** The `counter()` / `histogram()` handles and the auto
//!   per-command invocation/duration metrics described in spec 017 are **not yet
//!   exported** — no `MeterProvider` is installed and the OTLP `metrics` feature
//!   is not compiled. Calling these methods is safe but currently discards the
//!   values. Tracked as a follow-up; see the crate README. Prefer spans and
//!   [`Telemetry::event`] until metrics land.
//! - **[`SpanHandle::set_attr`]** can only record fields that were declared at
//!   the span's callsite (`tracing`'s fieldset is fixed per callsite), so
//!   arbitrary keys are dropped. [`SpanHandle::record_error`] works because its
//!   `otel.status_*` fields are pre-declared.
//! - Config fields `metrics_enabled`, `logs_enabled`, `record_arg_values`, and
//!   `arg_value_allowlist` are reserved for future signals and not yet consulted.
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
