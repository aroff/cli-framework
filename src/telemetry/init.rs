use crate::telemetry::{
    config::TelemetryConfig,
    exporter::RedactingExporter,
    guard::TelemetryGuard,
    handle::{Counter, CounterInner, Histogram, HistogramInner, SpanHandle, SpanInner, Telemetry},
    policy::TelemetryPolicy,
    redact::METRIC_LABEL_ALLOWLIST,
    resource::{
        apply_env_resource_attributes, metric_resource_attrs, to_resource, trace_resource_attrs,
        ServiceIdentity,
    },
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

/// Build the head-sampling `Sampler` for a resolved [`TelemetryPolicy`].
///
/// End-user Installs and `debug` always sample everything —
/// [`TelemetryPolicy::sampler_is_always_on`] is the single place that decides
/// so; everything else samples by `policy.sample_ratio`, already normalized to
/// `(0.0, 1.0]` by [`resolve_policy`](crate::telemetry::resolve_policy).
pub fn sampler_for_policy(policy: &TelemetryPolicy) -> opentelemetry_sdk::trace::Sampler {
    use opentelemetry_sdk::trace::Sampler;
    if policy.sampler_is_always_on() {
        return Sampler::AlwaysOn;
    }
    Sampler::ParentBased(Box::new(Sampler::TraceIdRatioBased(policy.sample_ratio)))
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
    use opentelemetry_otlp::{WithExportConfig, WithHttpConfig};
    let endpoint = config.endpoint.as_deref()?;
    let exporter = opentelemetry_otlp::MetricExporter::builder()
        .with_http()
        .with_endpoint(format!("{}/v1/metrics", endpoint.trim_end_matches('/')))
        .with_headers(config.headers.clone())
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

/// The closed metric-label allowlist as OTel [`Key`](opentelemetry::Key)s, for
/// [`StreamBuilder::with_allowed_attribute_keys`](opentelemetry_sdk::metrics::Stream).
///
/// Built fresh on every call rather than cached: it runs once per instrument
/// at meter-provider construction time, not per data point, so there is
/// nothing to optimize and a `OnceLock` would only add a second place this
/// list could drift from `METRIC_LABEL_ALLOWLIST`.
fn allowed_view_keys() -> Vec<opentelemetry::Key> {
    METRIC_LABEL_ALLOWLIST
        .iter()
        .map(|k| opentelemetry::Key::from_static_str(k))
        .collect()
}

/// Build the metrics pipeline for a resolved policy: the same OTLP
/// `PeriodicReader` as [`build_meter_provider`], but with a View that closes
/// every instrument's attribute keys to [`METRIC_LABEL_ALLOWLIST`] — the
/// metrics half of the export boundary, alongside [`RedactingExporter`] for
/// spans.
///
/// Unlike the span boundary, this has no policy-dependent behaviour: the
/// label allowlist is a fixed set, not one that varies by telemetry level, so
/// this takes no `&TelemetryPolicy` parameter.
fn build_meter_provider_from_policy(
    resource: opentelemetry_sdk::Resource,
    exporter: opentelemetry_otlp::MetricExporter,
) -> opentelemetry_sdk::metrics::SdkMeterProvider {
    use opentelemetry_sdk::metrics::{Instrument, SdkMeterProvider, Stream};
    SdkMeterProvider::builder()
        .with_reader(opentelemetry_sdk::metrics::PeriodicReader::builder(exporter).build())
        .with_resource(resource)
        .with_view(move |instrument: &Instrument| {
            Stream::builder()
                // `Instrument::name` borrows from the instrument, not
                // `'static`; `with_name` needs an owned value to satisfy
                // `Into<Cow<'static, str>>`.
                .with_name(instrument.name().to_string())
                .with_allowed_attribute_keys(allowed_view_keys())
                .build()
                .ok()
        })
        .build()
}

/// Exercise the metric-label allowlist directly, without building a whole
/// meter provider.
#[doc(hidden)]
pub fn view_keys_for_test(candidates: &[&str]) -> Vec<String> {
    candidates
        .iter()
        .filter(|k| crate::telemetry::redact::metric_label_is_allowed(k))
        .map(|k| k.to_string())
        .collect()
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
    use opentelemetry_otlp::{WithExportConfig, WithHttpConfig};
    let endpoint = config.endpoint.as_deref()?;
    opentelemetry_otlp::SpanExporter::builder()
        .with_http()
        .with_endpoint(format!("{}/v1/traces", endpoint.trim_end_matches('/')))
        .with_headers(config.headers.clone())
        .build()
        .ok()
}

/// Refuse to start on a protocol this crate cannot actually speak.
///
/// `OTEL_EXPORTER_OTLP_PROTOCOL=grpc` used to be parsed onto the config and then
/// ignored, exporting over HTTP regardless. That is the worst outcome: the
/// operator's explicit instruction is discarded silently, and the resulting
/// failure looks like a collector problem. Returning `None` here means telemetry
/// is off and says so, which is recoverable; guessing is not.
///
/// `eprintln!` rather than `tracing::warn!` for the same reason as
/// [`warn_subscriber_conflict`]: at this point we may not own the subscriber, so
/// a `tracing` event could be filtered out and never seen.
fn reject_unsupported_protocol(config: &TelemetryConfig) -> bool {
    if config.protocol_is_supported() {
        return false;
    }
    static ONCE: std::sync::Once = std::sync::Once::new();
    let protocol = config.protocol.clone();
    ONCE.call_once(move || {
        eprintln!(
            "cli-framework telemetry: OTEL_EXPORTER_OTLP_PROTOCOL is set to '{protocol}', which \
             this build cannot export with. Only '{supported}' is supported. Telemetry is \
             DISABLED rather than exported over a protocol you did not ask for.",
            supported = crate::telemetry::config::SUPPORTED_PROTOCOL,
        );
    });
    true
}

/// Build the tracer provider, honouring `traces_enabled`.
///
/// With traces disabled the provider is built **without an exporter** rather
/// than not built at all: spans are still created, so `cli.command` timings and
/// W3C context propagation to downstream services keep working, and only the
/// export is suppressed. Metrics are unaffected either way.
fn build_tracer_provider(
    config: &TelemetryConfig,
    resource: opentelemetry_sdk::Resource,
    batch: bool,
) -> Option<SdkTracerProvider> {
    let mut builder = SdkTracerProvider::builder()
        .with_sampler(sampler_for(config))
        .with_resource(resource);
    if config.traces_enabled {
        let exporter = span_exporter(config)?;
        builder = if batch {
            builder.with_batch_exporter(exporter)
        } else {
            builder.with_simple_exporter(exporter)
        };
    }
    Some(builder.build())
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
    if !config.is_active() || reject_unsupported_protocol(config) {
        return None;
    }
    let (svc, ver) = resolve_service(config, service_name, service_version);
    let resource = build_resource(svc, ver);
    let provider = build_tracer_provider(config, resource.clone(), false)?;
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
    if !config.is_active() || reject_unsupported_protocol(config) {
        return None;
    }
    let (svc, ver) = resolve_service(config, service_name, service_version);
    let resource = build_resource(svc, ver);
    let provider = build_tracer_provider(config, resource.clone(), true)?;
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

/// Build the OTLP span exporter for a policy's endpoint, wrapped in
/// [`RedactingExporter`] so a dropped probe's spans and disallowed
/// attributes never reach the wire. `None` when there is no endpoint to
/// export to, or the exporter cannot be constructed.
fn span_exporter_for_policy(
    policy: &Arc<TelemetryPolicy>,
) -> Option<RedactingExporter<opentelemetry_otlp::SpanExporter>> {
    use opentelemetry_otlp::WithExportConfig;
    let endpoint = policy.endpoint.as_deref()?;
    let exporter = opentelemetry_otlp::SpanExporter::builder()
        .with_http()
        .with_endpoint(format!("{}/v1/traces", endpoint.trim_end_matches('/')))
        .build()
        .ok()?;
    Some(RedactingExporter::new(exporter, policy.clone()))
}

/// Build the OTLP metric exporter for a policy's endpoint. `None` under the
/// same conditions as [`span_exporter_for_policy`].
fn metric_exporter_for_policy(
    policy: &TelemetryPolicy,
) -> Option<opentelemetry_otlp::MetricExporter> {
    use opentelemetry_otlp::WithExportConfig;
    let endpoint = policy.endpoint.as_deref()?;
    opentelemetry_otlp::MetricExporter::builder()
        .with_http()
        .with_endpoint(format!("{}/v1/metrics", endpoint.trim_end_matches('/')))
        .build()
        .ok()
}

/// Build the whole OTLP pipeline — traces and metrics — from a resolved
/// [`TelemetryPolicy`] rather than a [`TelemetryConfig`].
///
/// The redacting export boundary is wired in here, not left to the caller:
/// [`RedactingExporter`] filters spans, and the meter provider's View closes
/// every instrument's attributes to the metric-label allowlist. A policy
/// that resolves below [`TelemetryPolicy::exports`] never reaches an
/// exporter at all, and one that does cannot construct the pipeline a
/// different way that bypasses the boundary — Approach B, enforced at one
/// seam instead of at every callsite.
///
/// # Does not install a subscriber
///
/// Returns with `install: false` composed into the guard, the same choice
/// [`init_batch_without_subscriber`] makes and for the same reason: every
/// `#[cfg(test)] mod tests` block in this crate shares one test binary, so an
/// entry point that unconditionally calls [`install_subscriber`] is not
/// safely unit-testable without risking a global-subscriber race with
/// unrelated tests elsewhere in the crate. Compose [`otel_layer`] over the
/// returned guard into your own subscriber, the same as the other
/// `_without_subscriber` entry points.
pub fn init_from_policy(
    policy: Arc<TelemetryPolicy>,
    service: ServiceIdentity,
) -> Option<(Arc<dyn Telemetry + Send + Sync>, TelemetryGuard)> {
    if !policy.exports() {
        return None;
    }

    let raw_env_attrs = std::env::var("OTEL_RESOURCE_ATTRIBUTES").ok();

    let mut trace_attrs = trace_resource_attrs(&policy, &service);
    apply_env_resource_attributes(&mut trace_attrs, raw_env_attrs.as_deref());
    let trace_resource = to_resource(trace_attrs);

    let span_exporter = span_exporter_for_policy(&policy)?;
    let provider = SdkTracerProvider::builder()
        .with_sampler(sampler_for_policy(&policy))
        .with_resource(trace_resource)
        .with_batch_exporter(span_exporter)
        .build();

    let mut metric_attrs = metric_resource_attrs(&policy, &service);
    apply_env_resource_attributes(&mut metric_attrs, raw_env_attrs.as_deref());
    let metric_resource = to_resource(metric_attrs);

    let meter_provider = metric_exporter_for_policy(&policy)
        .map(|exporter| build_meter_provider_from_policy(metric_resource, exporter));

    Some(make_handle_and_guard(provider, meter_provider, false))
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

    /// Safe as a unit test because rejection happens *before* any global is
    /// touched — no subscriber or provider is installed on this path.
    #[test]
    fn init_refuses_an_unsupported_protocol() {
        let cfg = TelemetryConfig {
            endpoint: Some("http://127.0.0.1:9/".to_string()),
            protocol: "grpc".to_string(),
            ..Default::default()
        };
        assert!(
            cfg.is_active(),
            "fixture must otherwise be active, or this passes for the wrong reason"
        );
        assert!(
            init_batch(&cfg, "svc", "1.0").is_none(),
            "an unsupported protocol must disable telemetry loudly, not export \
             over HTTP anyway"
        );
    }

    /// The complementary half: the guard must not reject the normal case.
    #[test]
    fn init_accepts_the_supported_protocol() {
        let cfg = TelemetryConfig {
            endpoint: Some("http://127.0.0.1:9/".to_string()),
            ..Default::default()
        };
        assert!(!reject_unsupported_protocol(&cfg));
    }
}
