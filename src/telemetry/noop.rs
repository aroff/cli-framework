use crate::telemetry::handle::{
    Counter, CounterInner, Histogram, HistogramInner, SpanHandle, SpanInner, Telemetry,
};
use opentelemetry::KeyValue;

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
