// tests/unit/telemetry_pipeline.rs
use cli_framework::telemetry::{
    init_from_policy, sampler_for_policy, Deployment, FlushOutcome, ServiceIdentity,
    TelemetryConfig, TelemetryLevel,
};
use opentelemetry_sdk::trace::SpanExporter;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

mod support;
use support::policy_with;

/// How many of `count` distinct traces the policy's sampler actually samples.
///
/// This replaces a `format!("{:?}", sampler)` string match. A `Debug` impl is
/// not a contract — it belongs to the SDK and can change in a patch release —
/// and, the reason it matters here, a sampler that *prints* `AlwaysOn` while
/// dropping every span would have satisfied every assertion in this file.
/// Sampling is behaviour, so it is asserted by sampling.
///
/// Deterministic, not statistical: `TraceIdRatioBased` decides from the trace
/// id alone, so the same ids always produce the same count and nothing here
/// can flake.
fn sampled_out_of(policy: &cli_framework::telemetry::TelemetryPolicy, count: u64) -> usize {
    use opentelemetry::trace::{SamplingDecision, SpanKind, TraceId};
    use opentelemetry_sdk::trace::ShouldSample;

    let sampler = sampler_for_policy(policy);
    (0..count)
        .filter(|n| {
            // The SDK reads the *low* eight bytes of the trace id, so a
            // sequential counter would put every id in the sampled region and
            // make any ratio measure 100%. A fixed odd multiplier (the
            // golden-ratio constant) spreads consecutive counters across the
            // whole u64 range while staying completely reproducible.
            let mut bytes = [0u8; 16];
            bytes[8..].copy_from_slice(&n.wrapping_mul(0x9E37_79B9_7F4A_7C15).to_be_bytes());
            let result = sampler.should_sample(
                None,
                TraceId::from_bytes(bytes),
                "cli.command",
                &SpanKind::Internal,
                &[],
                &[],
            );
            result.decision == SamplingDecision::RecordAndSample
        })
        .count()
}

/// The number of traces every ratio assertion below is measured over. Large
/// enough that a wrong ratio cannot hide inside the tolerance, small enough to
/// stay instant.
const TRACES: u64 = 4_000;

/// `init_from_policy`, `init_with_exporter` and friends all call
/// `opentelemetry::global::set_tracer_provider`/`set_meter_provider` —
/// process-wide globals shared by every test in this binary. The default
/// test harness runs every `#[test]` in one process on separate threads, so
/// two such tests running concurrently could each stomp on the other's
/// provider. Serializing them behind one lock mirrors `support::EnvGuard`'s
/// rationale for the process environment, applied here to OpenTelemetry's
/// global state instead of `std::env`.
///
/// Callers take it with `unwrap_or_else(|e| e.into_inner())`, never `unwrap()`:
/// one panicking test would otherwise poison the mutex and make every later
/// test in this binary fail on the poison instead of on its own assertion,
/// hiding the failure that actually mattered behind a wall of noise. The
/// data under the lock is `()` — there is no invariant a panic could have
/// broken. `support::EnvGuard` takes the same view of its own lock.
fn otel_global_lock() -> &'static Mutex<()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
}

/// Does the sampler defer to an existing parent decision, rather than
/// re-rolling the ratio for every child span?
fn parent_decides(
    policy: &cli_framework::telemetry::TelemetryPolicy,
    parent_sampled: bool,
) -> bool {
    use opentelemetry::trace::{
        SamplingDecision, SpanContext, SpanId, SpanKind, TraceContextExt, TraceFlags, TraceId,
        TraceState,
    };
    use opentelemetry_sdk::trace::ShouldSample;

    // A trace id the ratio sampler would reject on its own, so a `true` result
    // can only have come from the parent.
    let mut bytes = [0u8; 16];
    bytes[8..].copy_from_slice(&u64::MAX.to_be_bytes());
    let trace_id = TraceId::from_bytes(bytes);

    let flags = if parent_sampled {
        TraceFlags::SAMPLED
    } else {
        TraceFlags::default()
    };
    let cx = opentelemetry::Context::current().with_remote_span_context(SpanContext::new(
        trace_id,
        SpanId::from_bytes([1, 2, 3, 4, 5, 6, 7, 8]),
        flags,
        true,
        TraceState::default(),
    ));

    sampler_for_policy(policy)
        .should_sample(
            Some(&cx),
            trace_id,
            "cli.command",
            &SpanKind::Internal,
            &[],
            &[],
        )
        .decision
        == SamplingDecision::RecordAndSample
}

#[test]
fn an_end_user_install_samples_every_trace() {
    let policy = policy_with(
        Deployment::EndUser { privacy_url: None },
        TelemetryLevel::Usage,
        |_| {},
    );
    assert_eq!(
        sampled_out_of(&policy, TRACES),
        TRACES as usize,
        "one person's handful of invocations is not a volume problem, and a \
         sampled-away trace is a support case nobody can answer"
    );
}

#[test]
fn a_service_samples_by_ratio() {
    let policy = policy_with(Deployment::Service, TelemetryLevel::Diagnostic, |p| {
        p.sample_ratio = 0.25;
    });
    let sampled = sampled_out_of(&policy, TRACES);
    let fraction = sampled as f64 / TRACES as f64;
    assert!(
        (0.22..=0.28).contains(&fraction),
        "a service asked for a quarter of its traces and got {sampled}/{TRACES} ({fraction:.3})"
    );

    // And the ratio has to be the *policy's*, not a constant that happens to
    // look plausible: a different ratio must produce a different count.
    let tenth = policy_with(Deployment::Service, TelemetryLevel::Diagnostic, |p| {
        p.sample_ratio = 0.1;
    });
    let tenth_sampled = sampled_out_of(&tenth, TRACES);
    assert!(
        tenth_sampled < sampled,
        "0.1 sampled {tenth_sampled} and 0.25 sampled {sampled}; the sampler is ignoring the \
         policy's ratio"
    );

    // A parent that already decided wins over the ratio — that is what
    // `ParentBased` is for, and a bare `TraceIdRatioBased` would break trace
    // continuity by dropping children of sampled parents.
    assert!(
        parent_decides(&policy, true),
        "a child of a sampled parent must be sampled regardless of the ratio"
    );
    assert!(
        !parent_decides(&policy, false),
        "a child of an unsampled parent must not be sampled"
    );
}

#[test]
fn debug_forces_full_sampling_even_on_a_service_with_a_low_ratio() {
    let policy = policy_with(Deployment::Service, TelemetryLevel::Debug, |p| {
        p.sample_ratio = 0.01;
    });
    assert_eq!(
        sampled_out_of(&policy, TRACES),
        TRACES as usize,
        "a debug session that drops the trace being debugged is worse than useless"
    );
}

#[test]
fn a_flush_that_finishes_inside_the_budget_reports_completion() {
    let outcome =
        cli_framework::telemetry::flush_within_for_test(Duration::from_millis(500), || {});
    assert_eq!(outcome, FlushOutcome::Completed);
}

#[test]
fn a_flush_that_overruns_the_budget_reports_a_timeout_rather_than_blocking() {
    let started = std::time::Instant::now();
    let outcome =
        cli_framework::telemetry::flush_within_for_test(Duration::from_millis(100), || {
            std::thread::sleep(Duration::from_secs(5))
        });
    assert_eq!(outcome, FlushOutcome::TimedOut);
    assert!(
        started.elapsed() < Duration::from_secs(2),
        "the budget must bound the wait: waited {:?}",
        started.elapsed()
    );
}

#[test]
fn the_metric_view_keeps_only_allowlisted_labels() {
    let kept = cli_framework::telemetry::view_keys_for_test(&[
        "command",
        "status",
        "cli.install.id",
        "path",
    ]);
    assert_eq!(kept, vec!["command".to_string(), "status".to_string()]);
}

/// A `SpanExporter` fake local to this file, mirroring
/// `telemetry_exporter.rs`'s `ProbeExporter` — deliberately not promoted to
/// `support::mod.rs`, to keep that shared file's blast radius small. Records
/// the name of every span it is asked to export, and every `Resource` it is
/// handed via `set_resource`, so a test can observe what a caller-supplied
/// exporter actually receives through `init_with_exporter`/
/// `init_with_exporter_config` without depending on a real collector.
#[derive(Debug, Default)]
struct CapturingExporter {
    span_names: Arc<Mutex<Vec<String>>>,
    resources: Arc<Mutex<Vec<opentelemetry_sdk::Resource>>>,
}

impl SpanExporter for CapturingExporter {
    fn export(
        &self,
        batch: Vec<opentelemetry_sdk::trace::SpanData>,
    ) -> impl std::future::Future<Output = opentelemetry_sdk::error::OTelSdkResult> + Send {
        let names = self.span_names.clone();
        async move {
            names
                .lock()
                .unwrap()
                .extend(batch.into_iter().map(|s| s.name.to_string()));
            Ok(())
        }
    }
    fn set_resource(&mut self, resource: &opentelemetry_sdk::Resource) {
        self.resources.lock().unwrap().push(resource.clone());
    }
}

#[test]
fn init_from_policy_returns_none_when_the_policy_does_not_export() {
    let policy = Arc::new(policy_with(
        Deployment::Service,
        TelemetryLevel::Off,
        |_| {},
    ));
    let service = ServiceIdentity {
        name: "svc".to_string(),
        version: "1.0".to_string(),
    };
    assert!(
        init_from_policy(policy, service).is_none(),
        "a policy at TelemetryLevel::Off must never build an exporter, let \
         alone touch OpenTelemetry's process-wide globals"
    );
}

#[tokio::test]
async fn init_from_policy_builds_a_working_handle_that_flushes_within_budget() {
    use tracing_subscriber::layer::SubscriberExt;

    let _lock = otel_global_lock().lock().unwrap_or_else(|e| e.into_inner());
    let policy = Arc::new(policy_with(
        Deployment::Service,
        TelemetryLevel::Usage,
        |p| {
            // A loopback port nothing listens on fails fast; the fixture's
            // default `http://collector:4318` is a hostname that has to fail DNS
            // resolution first, which is too slow against the tight flush
            // budget this test asserts on.
            p.endpoint = Some("http://127.0.0.1:9/".to_string());
        },
    ));
    let service = ServiceIdentity {
        name: "pipeline-test-service".to_string(),
        version: "9.9.9".to_string(),
    };

    let (handle, guard) = init_from_policy(policy, service)
        .expect("a Usage-level policy with an endpoint must export");

    handle.event("cli.command", &[]);
    handle.counter("cli.invocations").add(1, &[]);
    handle.histogram("cli.duration_ms").record(12.5, &[]);

    let subscriber =
        tracing_subscriber::registry().with(cli_framework::telemetry::init::otel_layer(&guard));
    tracing::subscriber::with_default(subscriber, || {
        let span = handle.span("layered.span", &[]);
        drop(span);
    });

    assert_eq!(
        guard.flush_within(Duration::from_secs(2)),
        FlushOutcome::Completed,
        "a collector nothing listens on must still fail fast enough that \
         shutdown never blocks the process"
    );
}

#[test]
fn init_with_exporter_delivers_a_span_named_by_the_fixed_otel_convention() {
    use tracing_subscriber::layer::SubscriberExt;

    let _lock = otel_global_lock().lock().unwrap_or_else(|e| e.into_inner());
    let exporter = CapturingExporter::default();
    let span_names = exporter.span_names.clone();

    let (handle, guard) =
        cli_framework::telemetry::init::init_with_exporter(exporter, "test-service");

    let subscriber =
        tracing_subscriber::registry().with(cli_framework::telemetry::init::otel_layer(&guard));
    tracing::subscriber::with_default(subscriber, || {
        let span = handle.span("custom-span-name", &[]);
        drop(span);
    });

    let names = span_names.lock().unwrap().clone();
    assert_eq!(
        names,
        vec!["app.span".to_string()],
        "LiveTelemetry::span always emits the fixed OTel span name \"app.span\"; \
         the caller-supplied name is carried only in the span.name field, never \
         as otel.name — got {names:?}"
    );
}

#[test]
fn init_with_exporter_config_builds_a_resource_from_the_configured_service_identity() {
    use tracing_subscriber::layer::SubscriberExt;

    let _lock = otel_global_lock().lock().unwrap_or_else(|e| e.into_inner());
    let exporter = CapturingExporter::default();
    let resources = exporter.resources.clone();
    let span_names = exporter.span_names.clone();
    let config = TelemetryConfig {
        service_name: Some("configured-svc".to_string()),
        service_version: Some("configured-ver".to_string()),
        ..Default::default()
    };

    let (handle, guard) = cli_framework::telemetry::init::init_with_exporter_config(
        exporter,
        &config,
        "fallback-name",
        "fallback-ver",
    );

    let subscriber =
        tracing_subscriber::registry().with(cli_framework::telemetry::init::otel_layer(&guard));
    tracing::subscriber::with_default(subscriber, || {
        let span = handle.span("whatever", &[]);
        drop(span);
    });

    let seen = resources.lock().unwrap();
    let resource = seen
        .last()
        .expect("building the provider must call the exporter's set_resource");
    let name = resource
        .get(&opentelemetry::Key::from_static_str("service.name"))
        .expect("service.name must be set on the resource");
    assert_eq!(
        name.as_str().to_string(),
        "configured-svc",
        "the config's service_name must win over the fallback name argument"
    );
    let version = resource
        .get(&opentelemetry::Key::from_static_str("service.version"))
        .expect("service.version must be set on the resource");
    assert_eq!(
        version.as_str().to_string(),
        "configured-ver",
        "the config's service_version must win over the fallback version argument"
    );

    assert_eq!(
        span_names.lock().unwrap().clone(),
        vec!["app.span".to_string()]
    );
}

#[tokio::test]
async fn init_batch_without_subscriber_builds_a_handle_and_leaves_the_subscriber_alone() {
    let _lock = otel_global_lock().lock().unwrap_or_else(|e| e.into_inner());
    let config = TelemetryConfig {
        endpoint: Some("http://localhost:14318".to_string()),
        ..Default::default()
    };

    let built =
        cli_framework::telemetry::init::init_batch_without_subscriber(&config, "svc", "1.0");
    let (_handle, guard) = built.expect(
        "an active config with an endpoint must build a handle even when the \
         caller composes its own subscriber",
    );
    drop(guard);
}

#[test]
fn init_simple_builds_a_handle_with_a_simple_span_processor() {
    let _lock = otel_global_lock().lock().unwrap_or_else(|e| e.into_inner());
    let config = TelemetryConfig {
        endpoint: Some("http://localhost:14318".to_string()),
        ..Default::default()
    };

    let built = cli_framework::telemetry::init::init_simple(&config, "svc", "1.0");
    assert!(
        built.is_some(),
        "an active config with a supported protocol and an endpoint must build a handle"
    );
}
