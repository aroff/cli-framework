use crate::telemetry::handle::{
    Counter, CounterInner, Histogram, HistogramInner, SpanHandle, SpanInner, Telemetry,
};
use opentelemetry::KeyValue;

/// Zero-cost [`Telemetry`] implementation whose methods do nothing.
///
/// Always compiled (independent of the `telemetry` feature) and used as the
/// default handle returned by [`AppContext::telemetry`](crate::app::AppContext::telemetry)
/// when telemetry is disabled or no OTLP endpoint is configured, so handler code
/// can call the API unconditionally.
pub struct NoopTelemetry;

impl Telemetry for NoopTelemetry {
    fn event(&self, _: &str, _: &[KeyValue]) {}
    fn counter(&self, _: &str) -> Counter {
        Counter(CounterInner::Noop)
    }
    fn histogram(&self, _: &str) -> Histogram {
        Histogram(HistogramInner::Noop)
    }
    fn span(&self, _: &str, _: &[KeyValue]) -> SpanHandle {
        SpanHandle(SpanInner::Noop)
    }
}
