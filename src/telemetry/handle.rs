// Re-export KeyValue so callers don't need the opentelemetry dep directly
pub use opentelemetry::KeyValue;

pub trait Telemetry: Send + Sync {
    fn event(&self, name: &str, attrs: &[KeyValue]);
    fn counter(&self, name: &str) -> Counter;
    fn histogram(&self, name: &str) -> Histogram;
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
