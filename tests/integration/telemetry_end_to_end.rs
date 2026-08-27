//! End-to-end proof that the documented consumer path actually exports.
//!
//! Every other telemetry test builds the `tracing-opentelemetry` bridge layer by
//! hand before asserting on it. That made the suite green while the library
//! shipped no bridge at all, so a real `AppBuilder` exported **nothing** — the
//! defect this file exists to prevent.
//!
//! The rule for anything added here: touch only public API a consumer would
//! touch (`AppBuilder` → `with_telemetry` → `run_with_args`). Never construct a
//! subscriber, a layer, or a provider directly — doing so re-creates the exact
//! blind spot.
//!
//! # Why this is one test and not three
//!
//! `with_telemetry` installs a *process-global* `tracing` subscriber, bound to
//! the first provider that wins the race. A second test in this binary would
//! quietly export to the first test's collector and assert against an empty one.
//! So: one process, one run, three assertions.

use cli_framework::app::{AppBuilder, AppContext};
use cli_framework::command::Command;
use cli_framework::spec::command_tree::CommandSpec;
use cli_framework::telemetry::TelemetryConfig;
use std::sync::{Arc, Mutex};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

struct ProbeCtx;
impl AppContext for ProbeCtx {}

/// A real registered command.
///
/// Deliberately **not** the built-in `version`: that short-circuits in
/// `run_with_args` (`cmd_id == "version" && registry.get("version").is_none()`)
/// and returns before the dispatch seam, so it opens no `cli.command` span and
/// emits no metrics. Asserting against it would pass a broken build.
fn probe_command() -> Command {
    Command {
        id: Arc::from("probe"),
        spec: Arc::new(CommandSpec {
            summary: "Probe command",
            ..Default::default()
        }),
        validator: None,
        expose_mcp: false,
        expose_chat: false,
        meta: None,
        visibility: None,
        execute: Arc::new(|_ctx, _args| Box::pin(async move { Ok(()) })),
    }
}

/// Paths the OTLP/HTTP exporters POST to, relative to the configured endpoint.
const TRACES: &str = "/v1/traces";
const METRICS: &str = "/v1/metrics";

#[tokio::test]
async fn app_builder_run_exports_spans_and_metrics() {
    let server = MockServer::start().await;
    for p in [TRACES, METRICS] {
        Mock::given(method("POST"))
            .and(path(p))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(b""))
            .mount(&server)
            .await;
    }

    let cfg = TelemetryConfig {
        endpoint: Some(server.uri()),
        ..Default::default()
    };

    let mut app = AppBuilder::new()
        .with_version("probeapp", "1.2.3")
        .register_command(probe_command())
        .unwrap()
        .with_telemetry(cfg)
        .build(ProbeCtx)
        .unwrap();

    // Keep any command output out of libtest's stdout.
    app.stdout_capture = Some(Arc::new(Mutex::new(Vec::new())));

    // Assertion 1: the CLI dispatch path must not panic.
    //
    // `run_with_args` is `async`. `init_simple`'s `SimpleSpanProcessor` exports
    // inline through `reqwest::blocking`, which panics with "Cannot drop a
    // runtime in a context where blocking is not allowed" on the first span
    // close inside a Tokio worker. This fails if that path is reinstated.
    app.run_with_args(vec!["probeapp".to_string(), "probe".to_string()])
        .await
        .expect("CLI dispatch panicked or errored under a Tokio runtime");

    // `run_with_args` drops the TelemetryGuard on the way out, force-flushing
    // both pipelines; the batch worker still needs a beat to land the POST.
    tokio::time::sleep(std::time::Duration::from_millis(750)).await;

    let hits: Vec<String> = server
        .received_requests()
        .await
        .unwrap_or_default()
        .iter()
        .map(|r| r.url.path().to_string())
        .collect();

    // Assertion 2: the `cli.command` span reached the collector.
    // Before the bridge landed this saw zero requests — the library never
    // installed a `tracing-opentelemetry` layer, so spans stopped at `tracing`.
    assert!(
        hits.iter().any(|p| p == TRACES),
        "AppBuilder::run_with_args exported no spans — the tracing->OTel bridge \
         is not installed. Collector saw: {hits:?}"
    );

    // Assertion 3: the auto per-command metrics reached the collector (spec 019).
    // Before this landed no `MeterProvider` was installed, so `global::meter()`
    // returned a no-op and every recorded value was silently discarded.
    assert!(
        hits.iter().any(|p| p == METRICS),
        "AppBuilder::run_with_args exported no metrics — no MeterProvider is \
         installed, so counters/histograms are discarded. Collector saw: {hits:?}"
    );
}
