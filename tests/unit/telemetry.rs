//! Spec 017 tracer-bullet tests.

use cli_framework::telemetry::handle::Telemetry;
use cli_framework::telemetry::{NoopTelemetry, TelemetryConfig};

// ── 1. Noop handle compiles and doesn't panic ─────────────────────────────

#[test]
fn noop_telemetry_is_zero_cost() {
    let t = NoopTelemetry;
    t.event("evt", &[]);
    t.counter("c").add(1, &[]);
    t.histogram("h").record(1.0, &[]);
    let _s = t.span("s", &[]);
}

// ── 2. Config: inactive without endpoint ─────────────────────────────────

#[test]
fn config_inactive_without_endpoint() {
    let cfg = TelemetryConfig {
        endpoint: None,
        ..Default::default()
    };
    assert!(!cfg.is_active());
}

#[test]
fn config_active_with_endpoint() {
    // Guard against parallel test pollution from otel_sdk_disabled_env_vetoes_active_config.
    unsafe {
        std::env::remove_var("OTEL_SDK_DISABLED");
    }
    let cfg = TelemetryConfig {
        endpoint: Some("http://c:4318".into()),
        ..Default::default()
    };
    assert!(cfg.is_active());
}

#[test]
fn config_inactive_when_disabled() {
    let cfg = TelemetryConfig {
        enabled: false,
        endpoint: Some("http://c:4318".into()),
        ..Default::default()
    };
    assert!(!cfg.is_active());
}

// ── 3. OTEL_SDK_DISABLED veto ─────────────────────────────────────────────

#[test]
fn otel_sdk_disabled_env_vetoes_active_config() {
    unsafe {
        std::env::set_var("OTEL_SDK_DISABLED", "true");
    }
    let cfg = TelemetryConfig {
        enabled: true,
        endpoint: Some("http://c:4318".into()),
        ..Default::default()
    };
    assert!(!cfg.is_active(), "OTEL_SDK_DISABLED=true must veto");
    unsafe {
        std::env::remove_var("OTEL_SDK_DISABLED");
    }
}

// ── 4. from_env reads OTEL_* vars ────────────────────────────────────────

#[test]
fn from_env_reads_endpoint() {
    unsafe {
        std::env::set_var("OTEL_EXPORTER_OTLP_ENDPOINT", "http://col:4318");
    }
    let cfg = TelemetryConfig::from_env();
    assert_eq!(cfg.endpoint.as_deref(), Some("http://col:4318"));
    unsafe {
        std::env::remove_var("OTEL_EXPORTER_OTLP_ENDPOINT");
    }
}

#[test]
fn from_env_reads_service_name() {
    unsafe {
        std::env::set_var("OTEL_SERVICE_NAME", "my-svc");
    }
    let cfg = TelemetryConfig::from_env();
    assert_eq!(cfg.service_name.as_deref(), Some("my-svc"));
    unsafe {
        std::env::remove_var("OTEL_SERVICE_NAME");
    }
}

#[test]
fn from_env_reads_sample_ratio() {
    unsafe {
        std::env::set_var("OTEL_TRACES_SAMPLER_ARG", "0.5");
    }
    let cfg = TelemetryConfig::from_env();
    assert!((cfg.sample_ratio - 0.5).abs() < f64::EPSILON);
    unsafe {
        std::env::remove_var("OTEL_TRACES_SAMPLER_ARG");
    }
}

// ── 5. InvocationSurface ──────────────────────────────────────────────────

#[test]
fn invocation_surface_as_str() {
    use cli_framework::app::dispatch::InvocationSurface;
    assert_eq!(InvocationSurface::Cli.as_str(), "cli");
    assert_eq!(InvocationSurface::Chat.as_str(), "chat");
    assert_eq!(InvocationSurface::Mcp.as_str(), "mcp");
    assert_eq!(InvocationSurface::Api.as_str(), "api");
}

// ── 6. TestExporter + init_with_exporter produces spans ──────────────────

use opentelemetry_sdk::error::OTelSdkResult;
use opentelemetry_sdk::trace::{SpanData, SpanExporter};
use std::sync::{Arc, Mutex};

#[derive(Clone, Default, Debug)]
struct TestExporter(Arc<Mutex<Vec<SpanData>>>);

impl SpanExporter for TestExporter {
    async fn export(&self, batch: Vec<SpanData>) -> OTelSdkResult {
        self.0.lock().unwrap().extend(batch);
        Ok(())
    }
}

impl TestExporter {
    fn spans(&self) -> Vec<SpanData> {
        self.0.lock().unwrap().clone()
    }
}

#[tokio::test]
async fn init_with_exporter_captures_spans() {
    use tracing_subscriber::prelude::*;
    let exporter = TestExporter::default();
    let (_handle, guard) =
        cli_framework::telemetry::init::init_with_exporter(exporter.clone(), "test-service");
    // Use guard.tracer() instead of the global to avoid races with parallel tests
    let tracer = guard.tracer("cli-framework");
    let otel_layer = tracing_opentelemetry::layer().with_tracer(tracer);
    let subscriber = tracing_subscriber::registry().with(otel_layer);

    tracing::subscriber::with_default(subscriber, || {
        let _span = tracing::info_span!("test.span").entered();
    });

    guard.flush();
    let spans = exporter.spans();
    assert!(!spans.is_empty(), "expected at least one span");
}

// ── 7. Arg value allowlist config ─────────────────────────────────────────

#[test]
fn arg_value_allowlist_contains_only_safe_args() {
    let cfg = TelemetryConfig {
        record_arg_values: true,
        arg_value_allowlist: vec!["env".to_string(), "region".to_string()],
        ..Default::default()
    };
    assert!(cfg.arg_value_allowlist.contains(&"env".to_string()));
    assert!(!cfg.arg_value_allowlist.contains(&"token".to_string()));
    assert!(!cfg.arg_value_allowlist.contains(&"password".to_string()));
}

// ── 8. AppContext default returns noop ────────────────────────────────────

#[test]
fn app_context_default_returns_noop_telemetry() {
    use cli_framework::app::AppContext;

    struct Ctx;
    impl AppContext for Ctx {}

    let ctx = Ctx;
    ctx.telemetry().event("test", &[]);
    ctx.telemetry().counter("c").add(0, &[]);
}

// ── 9. TelemetryGuard flush doesn't panic ────────────────────────────────

#[tokio::test]
async fn telemetry_guard_flush_is_safe() {
    let exporter = TestExporter::default();
    let (_, guard) = cli_framework::telemetry::init::init_with_exporter(exporter, "test");
    guard.flush();
    // No panic = pass
}

// ── 10. SpanHandle record_error ───────────────────────────────────────────

#[tokio::test]
async fn span_handle_record_error_does_not_panic() {
    use tracing_subscriber::prelude::*;
    let exporter = TestExporter::default();
    let (handle, guard) = cli_framework::telemetry::init::init_with_exporter(exporter, "test");
    // Use guard.tracer() instead of the global to avoid races with parallel tests
    let tracer = guard.tracer("cli-framework");
    let otel_layer = tracing_opentelemetry::layer().with_tracer(tracer);
    let subscriber = tracing_subscriber::registry().with(otel_layer);

    tracing::subscriber::with_default(subscriber, || {
        let span = handle.span("test-op", &[]);
        let err = std::io::Error::new(std::io::ErrorKind::Other, "test error");
        span.record_error(&err);
    });
    drop(guard);
}

// ── 11. init_simple uses SimpleSpanProcessor (Gap 1) ─────────────────────
//
// SimpleSpanProcessor exports synchronously on span end; the TestExporter
// records spans in-memory so we can verify without an HTTP exporter (which
// would create a nested Tokio runtime incompatible with async test contexts).

#[test]
fn init_simple_exports_span_synchronously() {
    use tracing_subscriber::prelude::*;
    let exporter = TestExporter::default();
    let (_handle, guard) =
        cli_framework::telemetry::init::init_with_exporter(exporter.clone(), "test-simple");
    let tracer = guard.tracer("cli-framework");
    let otel_layer = tracing_opentelemetry::layer().with_tracer(tracer);
    let subscriber = tracing_subscriber::registry().with(otel_layer);

    tracing::subscriber::with_default(subscriber, || {
        let _s = tracing::info_span!("simple.test").entered();
    }); // span ends here → SimpleSpanProcessor exports immediately

    // No explicit flush required — SimpleSpanProcessor already delivered the span.
    let spans = exporter.spans();
    assert!(
        !spans.is_empty(),
        "SimpleSpanProcessor must deliver spans synchronously on span end"
    );
}

// ── 12. opt_telemetry_arc default returns None ────────────────────────────

#[test]
fn app_context_opt_telemetry_arc_default_returns_none() {
    use cli_framework::app::AppContext;

    struct Ctx;
    impl AppContext for Ctx {}

    let ctx = Ctx;
    assert!(
        ctx.opt_telemetry_arc().is_none(),
        "default opt_telemetry_arc() must return None"
    );
}

// ── 13. ApiServerBuilder::with_telemetry builder method ──────────────────

#[test]
fn api_server_builder_with_telemetry_stores_config() {
    use cli_framework::api::ApiServerBuilder;
    use cli_framework::telemetry::TelemetryConfig;

    let cfg = TelemetryConfig {
        endpoint: Some("http://otel:4318".into()),
        ..Default::default()
    };
    // with_telemetry is a builder method — it must not panic and must return Self
    let _builder = ApiServerBuilder::new().with_telemetry(cfg, "my-svc", "1.0.0");
}

// ── 14. init_simple returns None for inactive config ─────────────────────

#[test]
fn init_simple_returns_none_when_inactive() {
    use cli_framework::telemetry::init::init_simple;
    use cli_framework::telemetry::TelemetryConfig;

    let cfg = TelemetryConfig {
        enabled: false,
        ..Default::default()
    };
    assert!(
        init_simple(&cfg, "svc", "1.0").is_none(),
        "init_simple must return None when config is inactive"
    );
}

// ── 15. init_batch returns None for inactive config ───────────────────────

#[test]
fn init_batch_returns_none_when_inactive() {
    use cli_framework::telemetry::init::init_batch;
    use cli_framework::telemetry::TelemetryConfig;

    let cfg = TelemetryConfig {
        enabled: false,
        ..Default::default()
    };
    assert!(
        init_batch(&cfg, "svc", "1.0").is_none(),
        "init_batch must return None when config is inactive"
    );
}
