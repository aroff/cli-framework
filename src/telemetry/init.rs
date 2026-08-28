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

/// Build the `tracing` → OpenTelemetry bridge layer for an existing guard.
///
/// **Only spans that pass through a subscriber carrying this layer are exported.**
/// `with_telemetry()` installs a subscriber containing it automatically; use this
/// instead when the application owns its own subscriber (see
/// [`install_subscriber`]'s conflict warning).
///
/// # The guard must come from a non-installing entry point
///
/// [`init_batch`] and [`init_simple`] install a subscriber themselves, so
/// composing their guard into a second `registry().…init()` would panic on the
/// duplicate global — the previous version of this example did exactly that.
/// Use [`init_batch_without_subscriber`], which builds the same OTLP pipeline
/// and leaves the subscriber to you:
///
/// ```ignore
/// let (handle, guard) =
///     init_batch_without_subscriber(&cfg, "svc", "1.0").expect("telemetry inactive");
/// tracing_subscriber::registry()
///     .with(my_fmt_layer)
///     .with(cli_framework::telemetry::init::otel_layer(&guard))
///     .init();
/// ```
pub fn otel_layer<S>(
    guard: &TelemetryGuard,
) -> tracing_opentelemetry::OpenTelemetryLayer<S, opentelemetry_sdk::trace::SdkTracer>
where
    S: tracing::Subscriber + for<'a> tracing_subscriber::registry::LookupSpan<'a>,
{
    tracing_opentelemetry::layer().with_tracer(guard.tracer(INSTRUMENTATION_SCOPE))
}

/// Instrumentation scope name used for both the tracer and the meter.
const INSTRUMENTATION_SCOPE: &str = "cli-framework";

/// Install a process-wide subscriber carrying the OTel bridge layer.
///
/// Returns `false` when a global subscriber already exists — in which case the
/// caller's spans will never reach the OTel SDK, so we say so loudly rather than
/// exporting nothing in silence (the v1 failure mode this replaces).
fn install_subscriber(tracer: opentelemetry_sdk::trace::SdkTracer) -> bool {
    use tracing_subscriber::layer::SubscriberExt;
    use tracing_subscriber::util::SubscriberInitExt;

    let filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info"));

    tracing_subscriber::registry()
        .with(filter)
        .with(tracing_subscriber::fmt::layer().with_writer(std::io::stderr))
        .with(tracing_opentelemetry::layer().with_tracer(tracer))
        .try_init()
        .is_ok()
}

/// Warn once per process that telemetry is configured but cannot export.
///
/// Deliberately `eprintln!` and not `tracing::warn!`: the whole problem is that
/// we do not control the subscriber, so a `tracing` event could itself be
/// filtered out and the operator would never learn why their collector is empty.
fn warn_subscriber_conflict() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| {
        eprintln!(
            "cli-framework telemetry: a global `tracing` subscriber is already installed, \
             so OpenTelemetry spans will NOT be exported. Compose \
             `cli_framework::telemetry::init::otel_layer(&guard)` into your own subscriber \
             instead of relying on `with_telemetry()` alone."
        );
    });
}

/// Build the metrics pipeline (OTLP exporter behind a `PeriodicReader`).
///
/// `None` when metrics are switched off or the exporter cannot be built; traces
/// still work in that case, and the handle's counters fall back to no-ops.
fn build_meter_provider(
    config: &TelemetryConfig,
    resource: opentelemetry_sdk::Resource,
) -> Option<opentelemetry_sdk::metrics::SdkMeterProvider> {
    if !config.metrics_enabled {
        return None;
    }
    use opentelemetry_otlp::WithExportConfig;
    let endpoint = config.endpoint.as_deref()?;
    let exporter = opentelemetry_otlp::MetricExporter::builder()
        .with_http()
        .with_endpoint(format!("{}/v1/metrics", endpoint.trim_end_matches('/')))
        .build()
        .ok()?;
    let reader = opentelemetry_sdk::metrics::PeriodicReader::builder(exporter).build();
    Some(
        opentelemetry_sdk::metrics::SdkMeterProvider::builder()
            .with_reader(reader)
            .with_resource(resource)
            .build(),
    )
}

/// Wire up globals and hand back the handle + guard.
///
/// `install` is `false` for the test entry points, which compose their own
/// subscriber and must not race on the process-wide one.
fn make_handle_and_guard(
    provider: SdkTracerProvider,
    meter_provider: Option<opentelemetry_sdk::metrics::SdkMeterProvider>,
    install: bool,
) -> (Arc<dyn Telemetry + Send + Sync>, TelemetryGuard) {
    use opentelemetry::trace::TracerProvider as _;

    // Before the tracer provider, and unconditionally — including on the
    // `install: false` test paths, which already set process globals. The OTel
    // default propagator is a no-op, so without this every `inject_context`
    // writes no header and every `extract_context` returns an empty context,
    // both without erroring.
    crate::telemetry::propagation::install();

    opentelemetry::global::set_tracer_provider(provider.clone());
    // Must precede `global::meter()` below, or the handle captures a no-op meter
    // and every `counter()`/`histogram()` call silently discards its values.
    if let Some(mp) = &meter_provider {
        opentelemetry::global::set_meter_provider(mp.clone());
    }

    if install && !install_subscriber(provider.tracer(INSTRUMENTATION_SCOPE)) {
        warn_subscriber_conflict();
    }

    let meter = opentelemetry::global::meter(INSTRUMENTATION_SCOPE);
    (
        Arc::new(LiveTelemetry::new(meter)),
        TelemetryGuard::new(provider, meter_provider),
    )
}

/// Build the OTLP span exporter for a config's endpoint.
fn span_exporter(config: &TelemetryConfig) -> Option<opentelemetry_otlp::SpanExporter> {
    use opentelemetry_otlp::WithExportConfig;
    let endpoint = config.endpoint.as_deref()?;
    opentelemetry_otlp::SpanExporter::builder()
        .with_http()
        .with_endpoint(format!("{}/v1/traces", endpoint.trim_end_matches('/')))
        .build()
        .ok()
}

/// Init with `SimpleSpanProcessor` — synchronous export on span end.
///
/// # Do not call this from an async context
///
/// `SimpleSpanProcessor` exports inline on span end, and the OTLP exporter is
/// built on `reqwest::blocking`, which spins up and drops its own runtime. Doing
/// that inside a Tokio worker panics with *"Cannot drop a runtime in a context
/// where blocking is not allowed"*. Use [`init_batch`] anywhere a runtime is
/// live — including [`AppBuilder::run_with_args`], which is `async`.
///
/// [`AppBuilder::run_with_args`]: crate::app::AppBuilder::run_with_args
pub fn init_simple(
    config: &TelemetryConfig,
    service_name: &str,
    service_version: &str,
) -> Option<(Arc<dyn Telemetry + Send + Sync>, TelemetryGuard)> {
    if !config.is_active() {
        return None;
    }
    let exporter = span_exporter(config)?;
    let (svc, ver) = resolve_service(config, service_name, service_version);
    let resource = build_resource(svc, ver);
    let provider = SdkTracerProvider::builder()
        .with_sampler(sampler_for(config))
        .with_simple_exporter(exporter)
        .with_resource(resource.clone())
        .build();
    let meter_provider = build_meter_provider(config, resource);
    Some(make_handle_and_guard(provider, meter_provider, true))
}

/// Init with `BatchSpanProcessor` — asynchronous export on a background worker.
///
/// The default for every entry point that runs inside a Tokio runtime (the async
/// CLI dispatch path and long-running servers alike). Buffered spans are flushed
/// by [`TelemetryGuard`] on drop, so short-lived processes do not lose them.
pub fn init_batch(
    config: &TelemetryConfig,
    service_name: &str,
    service_version: &str,
) -> Option<(Arc<dyn Telemetry + Send + Sync>, TelemetryGuard)> {
    init_batch_inner(config, service_name, service_version, true)
}

/// Same OTLP batch pipeline as [`init_batch`], but leaves the `tracing`
/// subscriber alone.
///
/// For applications that own their own subscriber: compose
/// [`otel_layer`] over the returned guard. Without this, the only way to obtain
/// a guard for a real OTLP exporter was an entry point that had already claimed
/// the global subscriber, which made `otel_layer` impossible to use as
/// documented.
pub fn init_batch_without_subscriber(
    config: &TelemetryConfig,
    service_name: &str,
    service_version: &str,
) -> Option<(Arc<dyn Telemetry + Send + Sync>, TelemetryGuard)> {
    init_batch_inner(config, service_name, service_version, false)
}

fn init_batch_inner(
    config: &TelemetryConfig,
    service_name: &str,
    service_version: &str,
    install: bool,
) -> Option<(Arc<dyn Telemetry + Send + Sync>, TelemetryGuard)> {
    if !config.is_active() {
        return None;
    }
    let exporter = span_exporter(config)?;
    let (svc, ver) = resolve_service(config, service_name, service_version);
    let resource = build_resource(svc, ver);
    let provider = SdkTracerProvider::builder()
        .with_sampler(sampler_for(config))
        .with_batch_exporter(exporter)
        .with_resource(resource.clone())
        .build();
    let meter_provider = build_meter_provider(config, resource);
    Some(make_handle_and_guard(provider, meter_provider, install))
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
    make_handle_and_guard(provider, None, false)
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
    make_handle_and_guard(provider, None, false)
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
