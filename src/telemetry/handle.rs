// Re-export KeyValue so callers don't need the opentelemetry dep directly
pub use opentelemetry::KeyValue;

/// Handle for emitting telemetry from command handlers.
///
/// Obtained via [`AppContext::telemetry`](crate::app::AppContext::telemetry).
/// The default implementation is [`NoopTelemetry`](crate::telemetry::NoopTelemetry);
/// a live handle is installed only when the `telemetry` feature is on and an
/// OTLP endpoint is configured. Traces and metrics are both exported; see the
/// [module docs](crate::telemetry) for the remaining limitations.
pub trait Telemetry: Send + Sync {
    /// Emit a point-in-time event on the current span.
    fn event(&self, name: &str, attrs: &[KeyValue]);
    /// Return a monotonic counter handle.
    ///
    /// Exported over OTLP when a `metrics_enabled` config with an endpoint is
    /// active; a safe no-op otherwise.
    fn counter(&self, name: &str) -> Counter;
    /// Return a value-distribution histogram handle.
    ///
    /// Exported over OTLP when a `metrics_enabled` config with an endpoint is
    /// active; a safe no-op otherwise.
    fn histogram(&self, name: &str) -> Histogram;
    /// Open a child span; it closes when the returned [`SpanHandle`] is dropped.
    fn span(&self, name: &str, attrs: &[KeyValue]) -> SpanHandle;
}

pub struct Counter(pub(crate) CounterInner);
pub(crate) enum CounterInner {
    Noop,
    #[cfg(feature = "telemetry")]
    Live(opentelemetry::metrics::Counter<u64>),
}
impl Counter {
    pub fn add(&self, value: u64, attrs: &[KeyValue]) {
        match &self.0 {
            CounterInner::Noop => {
                let _ = (value, attrs);
            }
            #[cfg(feature = "telemetry")]
            CounterInner::Live(c) => c.add(value, attrs),
        }
    }
}

pub struct Histogram(pub(crate) HistogramInner);
pub(crate) enum HistogramInner {
    Noop,
    #[cfg(feature = "telemetry")]
    Live(opentelemetry::metrics::Histogram<f64>),
}
impl Histogram {
    pub fn record(&self, value: f64, attrs: &[KeyValue]) {
        match &self.0 {
            HistogramInner::Noop => {
                let _ = (value, attrs);
            }
            #[cfg(feature = "telemetry")]
            HistogramInner::Live(h) => h.record(value, attrs),
        }
    }
}

pub struct SpanHandle(pub(crate) SpanInner);
pub(crate) enum SpanInner {
    Noop,
    #[cfg(feature = "telemetry")]
    Live(tracing::span::EnteredSpan),
}
impl SpanHandle {
    /// Record an attribute on the span.
    ///
    /// Limitation: because the underlying `tracing` span has a fixed fieldset
    /// fixed at its callsite, only pre-declared keys take effect — arbitrary
    /// keys are silently dropped. [`record_error`](Self::record_error) works
    /// because its fields are declared up front.
    pub fn set_attr(&self, key: &'static str, value: &str) {
        match &self.0 {
            SpanInner::Noop => {
                let _ = (key, value);
            }
            #[cfg(feature = "telemetry")]
            SpanInner::Live(s) => {
                s.record(key, value);
            }
        }
    }
    /// Mark the span as errored, setting its OTel status to `Error` with the
    /// error's message as the description.
    pub fn record_error(&self, err: &dyn std::error::Error) {
        match &self.0 {
            SpanInner::Noop => {
                let _ = err;
            }
            #[cfg(feature = "telemetry")]
            SpanInner::Live(s) => {
                s.record("otel.status_code", "ERROR");
                s.record("otel.status_description", &*err.to_string());
            }
        }
    }
}
