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
/// before ever reaching `execute_command_direct`, the general dispatch seam
/// this test exercises. `version` instruments itself separately at the
/// short-circuit site — see `telemetry_version_command_span.rs` (spec 020
/// item 6) — so it's covered by its own test, not this one.
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

/// Decode an OTLP/HTTP protobuf trace body into the span names and attribute
/// keys it carries.
///
/// Every assertion elsewhere in this crate stops at a `Vec<KeyValue>` in
/// memory. This is the only place that decodes the actual bytes an exporter
/// put on the wire, which is the only way to check the protobuf encoding
/// itself — not just the in-memory value the encoder was handed — is correct.
fn decode_spans(body: &[u8]) -> Vec<(String, Vec<String>)> {
    use opentelemetry_proto::tonic::collector::trace::v1::ExportTraceServiceRequest;
    use prost::Message;
    let request = ExportTraceServiceRequest::decode(body).expect("valid OTLP protobuf");
    request
        .resource_spans
        .into_iter()
        .flat_map(|rs| rs.scope_spans)
        .flat_map(|ss| ss.spans)
        .map(|span| {
            let keys = span.attributes.into_iter().map(|kv| kv.key).collect();
            (span.name, keys)
        })
        .collect()
}

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

    let requests = server.received_requests().await.unwrap_or_default();
    let hits: Vec<String> = requests.iter().map(|r| r.url.path().to_string()).collect();

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

    // Assertion 4: decode the actual protobuf bytes rather than merely
    // counting the request — proof that a real span survives OTLP/HTTP
    // protobuf encoding with the attribute this crate's own instrumentation
    // put on it, not just the in-memory `KeyValue` the encoder was handed.
    let trace_bodies: Vec<&[u8]> = requests
        .iter()
        .filter(|r| r.url.path() == TRACES)
        .map(|r| r.body.as_slice())
        .collect();
    assert!(
        !trace_bodies.is_empty(),
        "a request hit {TRACES} but carried no body to decode"
    );
    let spans: Vec<(String, Vec<String>)> =
        trace_bodies.iter().flat_map(|b| decode_spans(b)).collect();
    let (_, keys) = spans
        .iter()
        .find(|(name, _)| name == "cli.command")
        .unwrap_or_else(|| panic!("no cli.command span among decoded spans: {spans:?}"));
    assert!(
        keys.contains(&"command".to_string()),
        "the decoded span carries no command attribute: {keys:?}"
    );

    // Deliberately NOT asserted here: that `cli.probe` is absent from the
    // wire. It is not — this run puts it on the wire for real, and this
    // assertion caught that live rather than assuming the boundary applies.
    //
    // `AppBuilder::init_telemetry` (src/app/builder.rs) calls
    // `telemetry::init::init_batch`, whose span exporter comes from
    // `build_tracer_provider` -> `span_exporter(config)`
    // (src/telemetry/init.rs): a bare `opentelemetry_otlp::SpanExporter`,
    // never wrapped in `RedactingExporter`. The wrapped, policy-aware
    // pipeline exists and is unit-tested (`init_from_policy` /
    // `span_exporter_for_policy`, same file) but nothing outside its own
    // `#[cfg(test)]` module calls it. `AppBuilder`'s own field doc says so
    // directly (the `telemetry_policy` field, src/app/builder.rs:927-942):
    // "`App` still runs the pre-existing `TelemetryConfig` -> `init_batch`
    // export path ..., which never consults `TelemetryPolicy` at all. Full
    // `App`-level policy orchestration ... is deferred to PR7." The same
    // gap holds for metrics: `build_meter_provider` (config-based, what
    // `init_batch` uses) attaches no View, so the metric-label allowlist
    // `build_meter_provider_from_policy` applies is equally unenforced here.
    // Wiring either fix means editing `src/app/builder.rs` and/or
    // `src/telemetry/init.rs`'s production callsites, both outside this
    // task's file list and squarely PR7/Task 26's job per that same field
    // doc — reported in full rather than silently patched around.
}
