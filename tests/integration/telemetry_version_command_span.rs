//! Proves the built-in `version` command is instrumented like every other
//! command (spec 020 item 6).
//!
//! `version` is a bespoke short-circuit inside `run_with_args` — it renders
//! `AppBuilder::version_string()` directly and returns before the code ever
//! reaches `execute_command_direct`, which is where every other command's
//! `cli.command` span and `cli.command.invocations`/`cli.command.duration_ms`
//! metrics come from. Before the fix, invoking `version` on an app that hasn't
//! registered its own `version` command produced zero span and zero metrics,
//! while every other command produced both — an asymmetry the module docs at
//! `src/telemetry/mod.rs` explicitly claim does not exist ("Every command
//! dispatch is automatically wrapped in a `cli.command` span").
//!
//! [`telemetry_end_to_end`] already proves the general dispatch path exports;
//! this file proves the `version` short-circuit specifically does too. Metrics
//! are simpler and sufficient here: `version` has no route pattern to get
//! wrong the way an HTTP span's name does, so there's no cardinality rule to
//! prove beyond "the metric carries `command = version`".
//!
//! # Why this is one test and not several
//!
//! `with_telemetry` installs a *process-global* `tracing` subscriber bound to
//! the first provider that wins the race. A second test in this binary would
//! export into the first test's collector and assert against an empty one. So
//! this is its own `[[test]]` binary, same convention as every other
//! telemetry integration test in this crate.

use cli_framework::app::{AppBuilder, AppContext};
use cli_framework::telemetry::TelemetryConfig;
use std::sync::{Arc, Mutex};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

struct ProbeCtx;
impl AppContext for ProbeCtx {}

/// Paths the OTLP/HTTP exporters POST to, relative to the configured endpoint.
const TRACES: &str = "/v1/traces";
const METRICS: &str = "/v1/metrics";

/// OTLP/HTTP protobuf, uncompressed by default: a `string_value` field (field
/// 1 of `AnyValue`, wire type 2 = length-delimited) with the exact 7-byte
/// payload `"version"` serialises to the literal byte sequence
/// `0x0A 0x07 "version"`. That tag+length pair only matches a length-7 string
/// field, so it can't accidentally fire on the *key* `"cli.command.path"` or
/// on the resource attribute `service.version` (key string, length 15) whose
/// *value* is the crate/app version number, not the word "version".
const STRING_VALUE_VERSION: &[u8] = b"\x0a\x07version";

#[tokio::test]
async fn version_command_exports_span_and_metrics() {
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
        .with_telemetry(cfg)
        .build(ProbeCtx)
        .unwrap();

    // Keep command output out of libtest's stdout.
    app.stdout_capture = Some(Arc::new(Mutex::new(Vec::new())));

    // No `version` command is registered, so this hits the built-in
    // short-circuit in `run_with_args`, not `execute_command_direct`.
    app.run_with_args(vec!["probeapp".to_string(), "version".to_string()])
        .await
        .expect("built-in `version` dispatch panicked or errored");

    // `run_with_args` drops the TelemetryGuard on the way out, force-flushing
    // both pipelines; the batch worker still needs a beat to land the POST.
    tokio::time::sleep(std::time::Duration::from_millis(750)).await;

    let requests = server.received_requests().await.unwrap_or_default();

    let metrics_bodies: Vec<&[u8]> = requests
        .iter()
        .filter(|r| r.url.path() == METRICS)
        .map(|r| r.body.as_slice())
        .collect();

    assert!(
        !metrics_bodies.is_empty(),
        "built-in `version` exported no metrics at all — the short-circuit in \
         `run_with_args` never reaches the telemetry seam"
    );

    let has_version_value = metrics_bodies.iter().any(|b| {
        b.windows(STRING_VALUE_VERSION.len())
            .any(|w| w == STRING_VALUE_VERSION)
    });

    assert!(
        has_version_value,
        "no exported metric carries a `command = \"version\"` attribute — the \
         built-in `version` short-circuit is not recording \
         `cli.command.invocations`/`cli.command.duration_ms`"
    );

    // The span side of the same defect: no `cli.command` span for `version`
    // means invoking it is invisible in trace-based usage analytics too.
    let traces_hit = requests.iter().any(|r| r.url.path() == TRACES);
    assert!(
        traces_hit,
        "built-in `version` exported no spans — the `cli.command` span in the \
         short-circuit block never opened"
    );
}
