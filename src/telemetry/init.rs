use crate::telemetry::{
    config::TelemetryConfig,
    guard::TelemetryGuard,
    handle::{Counter, CounterInner, Histogram, HistogramInner, SpanHandle, SpanInner, Telemetry},
};
use opentelemetry::KeyValue;
use opentelemetry_sdk::trace::SdkTracerProvider;
use std::sync::Arc;

pub struct LiveTelemetry {
    meter: opentelemetry::metrics::Meter,
}

impl LiveTelemetry {
    pub fn new(meter: opentelemetry::metrics::Meter) -> Self {
        Self { meter }
    }
}

impl Telemetry for LiveTelemetry {
    fn event(&self, name: &str, attrs: &[KeyValue]) {
        tracing::info!(telemetry.event.name = name, attrs = ?attrs);
    }
    fn counter(&self, name: &str) -> Counter {
        Counter(CounterInner::Live(
            self.meter.u64_counter(name.to_string()).build(),
        ))
    }
    fn histogram(&self, name: &str) -> Histogram {
        Histogram(HistogramInner::Live(
            self.meter.f64_histogram(name.to_string()).build(),
        ))
    }
    fn span(&self, name: &str, attrs: &[KeyValue]) -> SpanHandle {
        let span = tracing::info_span!("app.span", span.name = name, attrs = ?attrs);
        SpanHandle(SpanInner::Live(span.entered()))
    }
}

fn build_resource(service_name: &str, service_version: &str) -> opentelemetry_sdk::Resource {
    opentelemetry_sdk::Resource::builder_empty()
        .with_attributes(vec![
            KeyValue::new("service.name", service_name.to_string()),
            KeyValue::new("service.version", service_version.to_string()),
        ])
        .build()
}

fn make_handle_and_guard(
    provider: SdkTracerProvider,
) -> (Arc<dyn Telemetry + Send + Sync>, TelemetryGuard) {
    opentelemetry::global::set_tracer_provider(provider.clone());
    let meter = opentelemetry::global::meter("cli-framework");
    (
        Arc::new(LiveTelemetry::new(meter)),
        TelemetryGuard::new(provider),
    )
}

/// Init with SimpleSpanProcessor for one-shot CLI runs.
///
/// `SimpleSpanProcessor` exports synchronously on span end — lossless for
/// short-lived processes that may exit before an async batch would flush.
pub fn init_simple(
    config: &TelemetryConfig,
    service_name: &str,
    service_version: &str,
) -> Option<(Arc<dyn Telemetry + Send + Sync>, TelemetryGuard)> {
    if !config.is_active() {
        return None;
    }
    use opentelemetry_otlp::WithExportConfig;
    let endpoint = config.endpoint.as_deref().unwrap();
    let exporter = opentelemetry_otlp::SpanExporter::builder()
        .with_http()
        .with_endpoint(format!("{}/v1/traces", endpoint.trim_end_matches('/')))
        .build()
        .ok()?;
    let provider = SdkTracerProvider::builder()
        .with_simple_exporter(exporter)
        .with_resource(build_resource(service_name, service_version))
        .build();
    Some(make_handle_and_guard(provider))
}

/// Init with BatchSpanProcessor for long-running servers.
pub fn init_batch(
    config: &TelemetryConfig,
    service_name: &str,
    service_version: &str,
) -> Option<(Arc<dyn Telemetry + Send + Sync>, TelemetryGuard)> {
    if !config.is_active() {
        return None;
    }
    use opentelemetry_otlp::WithExportConfig;
    let endpoint = config.endpoint.as_deref().unwrap();
    let exporter = opentelemetry_otlp::SpanExporter::builder()
        .with_http()
        .with_endpoint(format!("{}/v1/traces", endpoint.trim_end_matches('/')))
        .build()
        .ok()?;
    let provider = SdkTracerProvider::builder()
        .with_batch_exporter(exporter)
        .with_resource(build_resource(service_name, service_version))
        .build();
    Some(make_handle_and_guard(provider))
}

/// Init with a custom SpanExporter — used in tests.
pub fn init_with_exporter(
    exporter: impl opentelemetry_sdk::trace::SpanExporter + 'static,
    service_name: &str,
) -> (Arc<dyn Telemetry + Send + Sync>, TelemetryGuard) {
    let resource = opentelemetry_sdk::Resource::builder_empty()
        .with_attributes(vec![KeyValue::new(
            "service.name",
            service_name.to_string(),
        )])
        .build();
    let provider = SdkTracerProvider::builder()
        .with_simple_exporter(exporter)
        .with_resource(resource)
        .build();
    make_handle_and_guard(provider)
}
