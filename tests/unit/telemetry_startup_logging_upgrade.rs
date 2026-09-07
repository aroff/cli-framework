// tests/unit/telemetry_startup_logging_upgrade.rs
//
//! One test, one process: `init_default_logging()` in `main`, telemetry
//! afterwards.
//!
//! This is the case the attach slot exists for. An application calls
//! `cli_framework::init_default_logging()` to get human-readable logs, which
//! claims the process-global subscriber; startup then arrives and finds one
//! already installed. Reporting `ForeignSubscriber` there would mean the
//! framework's *own* recommended `main` silently exports no traces.
//!
//! # Why this replaces the plan's test
//!
//! The plan's `startup_returns_a_logging_guard_that_the_caller_must_hold`
//! reads `let _guard = result.logging;` and asserts nothing — it passes
//! against an implementation that returns a guard and attaches nothing, which
//! is exactly the bug worth catching. The assertion below is on what the
//! attached layer *does*: a span opened through the global dispatcher must
//! carry a valid OpenTelemetry span context, which it can only get from the
//! OTel bridge layer having been swapped into the live subscriber.

use cli_framework::config::resolution::Layer;
use cli_framework::config::ConfigFormat;
use cli_framework::telemetry::{
    run_startup, telemetry_only_manifest, Deployment, ProbeRegistry, ServiceIdentity,
    StartupInputs, SubscriberOutcome, Surface, TelemetryInputs, TelemetryStoreLocation,
};
use tracing_opentelemetry::OpenTelemetrySpanExt;

#[test]
fn default_logging_is_upgraded_with_the_otel_layer_instead_of_being_reported_foreign() {
    // The composed subscriber filters on `RUST_LOG`; pin it so an operator's
    // environment cannot turn the assertion below off.
    // SAFETY: single-threaded test setup, before any span is opened.
    unsafe {
        std::env::set_var("RUST_LOG", "info");
    }

    let logging = cli_framework::init_default_logging();
    assert!(
        logging.can_attach_otel_layer(),
        "this test owns the process global, so the guard must hold a live slot"
    );

    let dir = tempfile::tempdir().expect("a temp dir");
    let registry = ProbeRegistry::with_builtins();
    let manifest = telemetry_only_manifest("demo", &registry, None);
    let inputs = StartupInputs {
        base: TelemetryInputs {
            app: "demo".to_string(),
            deployment: Deployment::Service,
            endpoint: Some("http://127.0.0.1:9/".to_string()),
            endpoint_source: Some(Layer::Default),
            session_id: "session-fixture".to_string(),
            registry,
            ..Default::default()
        },
        store: TelemetryStoreLocation {
            dir: Some(dir.path().to_path_buf()),
            format: ConfigFormat::Json,
        },
        manifest: std::sync::Arc::new(manifest),
        service: ServiceIdentity {
            name: "demo".to_string(),
            version: "0.0.0".to_string(),
        },
        surface: Surface::Cli,
        stderr_is_tty: false,
        env: Vec::new(),
    };

    let result = run_startup(inputs);

    assert!(result.policy.exports(), "the fixture must export");
    assert_eq!(
        result.report.subscriber,
        SubscriberOutcome::Installed,
        "`init_default_logging` leaves an attach slot precisely so telemetry can \
         upgrade it; treating it as a foreign subscriber would mean the \
         framework's own recommended `main` exports nothing"
    );
    assert!(
        !result
            .report
            .findings
            .iter()
            .any(|f| f.check_id == "telemetry.subscriber"),
        "an upgraded subscriber is not a degraded one and must not raise a \
         doctor finding"
    );

    let span = tracing::info_span!("cli.command");
    let context = span.context();
    // Bound, not chained: `TraceContextExt::span` hands back a `SpanRef`
    // borrowed from `context`, and `span_context()` borrows from *that*.
    // Chaining drops the `SpanRef` at the end of the statement.
    let span_ref = opentelemetry::trace::TraceContextExt::span(&context);
    assert!(
        span_ref.span_context().is_valid(),
        "the span carries no OpenTelemetry context, so the bridge layer was \
         never swapped into the live subscriber and nothing this process \
         traces will ever reach the collector"
    );

    drop(result.guard);
    drop(logging);
}
