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
        // `otel.status_code` / `otel.status_description` are special fields that
        // `tracing-opentelemetry` maps onto the OTel span *status*. They must be
        // declared here as `Empty` — `tracing::Span::record` only affects fields
        // present in the callsite's fieldset, so `record_error` is a silent no-op
        // unless the fields exist up front.
        let span = tracing::info_span!(
            "app.span",
            span.name = name,
            attrs = ?attrs,
            otel.status_code = tracing::field::Empty,
            otel.status_description = tracing::field::Empty,
        );
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

/// Resolve the effective service name/version for the OTel resource.
///
/// A value carried on the config (e.g. `OTEL_SERVICE_NAME`, read by
/// [`TelemetryConfig::from_env`]) takes precedence over the caller-supplied
/// default (typically the app's own name/version). Without this, the config
/// fields would be silently ignored and `OTEL_SERVICE_NAME` would have no effect.
fn resolve_service<'a>(
    config: &'a TelemetryConfig,
    service_name: &'a str,
    service_version: &'a str,
) -> (&'a str, &'a str) {
    (
        config.service_name.as_deref().unwrap_or(service_name),
        config.service_version.as_deref().unwrap_or(service_version),
    )
}

/// Build the head-sampling `Sampler` for a config's `sample_ratio`.
///
/// `ParentBased` so that a sampled parent keeps its children; the root decision
/// is ratio-based. `ratio >= 1.0` keeps everything (the default).
fn sampler_for(config: &TelemetryConfig) -> opentelemetry_sdk::trace::Sampler {
    use opentelemetry_sdk::trace::Sampler;
    Sampler::ParentBased(Box::new(Sampler::TraceIdRatioBased(config.sample_ratio)))
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
    let (svc, ver) = resolve_service(config, service_name, service_version);
    let provider = SdkTracerProvider::builder()
        .with_sampler(sampler_for(config))
        .with_simple_exporter(exporter)
        .with_resource(build_resource(svc, ver))
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
    let (svc, ver) = resolve_service(config, service_name, service_version);
    let provider = SdkTracerProvider::builder()
        .with_sampler(sampler_for(config))
        .with_batch_exporter(exporter)
        .with_resource(build_resource(svc, ver))
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

/// Init with a custom SpanExporter, honouring the config's sampler and service
/// name/version resolution. Lets tests exercise those paths (which the OTLP
/// [`init_simple`]/[`init_batch`] entry points also use) with an in-memory
/// exporter instead of a live collector.
#[doc(hidden)]
pub fn init_with_exporter_config(
    exporter: impl opentelemetry_sdk::trace::SpanExporter + 'static,
    config: &TelemetryConfig,
    service_name: &str,
    service_version: &str,
) -> (Arc<dyn Telemetry + Send + Sync>, TelemetryGuard) {
    let (svc, ver) = resolve_service(config, service_name, service_version);
    let provider = SdkTracerProvider::builder()
        .with_sampler(sampler_for(config))
        .with_simple_exporter(exporter)
        .with_resource(build_resource(svc, ver))
        .build();
    make_handle_and_guard(provider)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_service_prefers_config_over_default() {
        let cfg = TelemetryConfig {
            service_name: Some("cfg-svc".into()),
            service_version: Some("cfg-ver".into()),
            ..Default::default()
        };
        assert_eq!(
            resolve_service(&cfg, "app-name", "app-ver"),
            ("cfg-svc", "cfg-ver")
        );
    }

    #[test]
    fn resolve_service_falls_back_to_default_when_config_unset() {
        let cfg = TelemetryConfig::default();
        assert_eq!(
            resolve_service(&cfg, "app-name", "app-ver"),
            ("app-name", "app-ver")
        );
    }
}
