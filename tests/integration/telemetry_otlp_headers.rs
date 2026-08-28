//! Proof that `TelemetryConfig::headers` reaches the collector (spec 017 R25).
//!
//! # What was actually broken, and what was not
//!
//! Spec 020 recorded R25 as "`OTEL_EXPORTER_OTLP_HEADERS` is not read, so a
//! collector requiring authentication cannot be reached at all". **That
//! overstates it.** `opentelemetry-otlp` reads that environment variable itself
//! (`exporter/http/mod.rs`), so an env-configured deployment was already
//! authenticating.
//!
//! The real gap was narrower: headers set **programmatically** on
//! `TelemetryConfig` — by an app reading them from its own config file, a secret
//! store, or anywhere that is not the process environment — were parsed onto the
//! struct and then dropped on the floor. That is what `with_headers` on both
//! exporter builders fixes, and what this test covers.
//!
//! # Why this test deliberately does NOT set the env var
//!
//! The first version of this test set `OTEL_EXPORTER_OTLP_HEADERS` and asserted
//! the header arrived. It passed — and kept passing with `with_headers` removed
//! from the exporter, because the SDK was supplying the header from the
//! environment the whole time. The test proved the SDK worked, not this crate.
//!
//! So the variable is explicitly REMOVED below. The only possible source of the
//! header on the wire is `TelemetryConfig::headers`, which makes a mutation of
//! the call site fail the way it should.
//!
//! # Why this is its own test binary
//!
//! `init_batch` installs a *process-global* subscriber and tracer provider, and
//! this test mutates process environment.

use cli_framework::telemetry::TelemetryConfig;
use std::collections::HashMap;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

const TRACES: &str = "/v1/traces";
const METRICS: &str = "/v1/metrics";

#[tokio::test]
async fn config_headers_are_sent_to_the_collector() {
    let collector = MockServer::start().await;
    for p in [TRACES, METRICS] {
        Mock::given(method("POST"))
            .and(path(p))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(b""))
            .mount(&collector)
            .await;
    }

    // The load-bearing line: with this set, the SDK would supply the header on
    // its own and this test would pass against a broken exporter config.
    // SAFETY: single-threaded test setup, before any task is spawned.
    unsafe {
        std::env::remove_var("OTEL_EXPORTER_OTLP_HEADERS");
    }

    let mut headers = HashMap::new();
    headers.insert("authorization".to_string(), "Bearer tok-abc".to_string());
    headers.insert("x-scope-orgid".to_string(), "acme".to_string());

    let cfg = TelemetryConfig {
        endpoint: Some(collector.uri()),
        headers,
        ..Default::default()
    };

    let (telemetry, guard) =
        cli_framework::telemetry::init::init_batch(&cfg, "header-probe", "1.0.0")
            .expect("telemetry should initialise against the stub collector");

    tracing::info_span!("probe.span").in_scope(|| {
        tracing::info!("probe");
    });

    // Both signals, on purpose. The span and metric exporters are configured by
    // separate builders, so a fix applied to one and missed on the other has to
    // be catchable — and it is not unless a metric is actually recorded here.
    // `force_flush` on a meter provider with nothing in it POSTs nothing, which
    // silently reduced the metrics half of this test to decoration.
    telemetry
        .counter("probe.counter")
        .add(1, &[opentelemetry::KeyValue::new("probe", "headers")]);

    guard.flush();
    tokio::time::sleep(std::time::Duration::from_millis(750)).await;

    let requests = collector.received_requests().await.unwrap_or_default();
    assert!(
        !requests.is_empty(),
        "collector received nothing, so this proves nothing about headers"
    );

    // Pinned to /v1/traces specifically. Accepting "any request that carried the
    // header" would let a regression in the SPAN exporter pass whenever the
    // metrics exporter happened to flush first — the two are configured
    // separately and each has to be checked.
    let req = requests
        .iter()
        .find(|r| r.url.path() == TRACES)
        .unwrap_or_else(|| {
            panic!(
                "no /v1/traces request arrived, so this proves nothing. Paths seen: {:?}",
                requests.iter().map(|r| r.url.path()).collect::<Vec<_>>()
            )
        });

    let auth = req.headers.get("authorization").unwrap_or_else(|| {
        panic!(
            "the /v1/traces request carried no `authorization` header. \
             `TelemetryConfig::headers` was populated but never handed to the SPAN \
             exporter builder, so a programmatically-configured credential never \
             reaches the collector."
        )
    });
    assert_eq!(auth, "Bearer tok-abc");
    assert_eq!(
        req.headers
            .get("x-scope-orgid")
            .map(|v| v.to_str().unwrap()),
        Some("acme"),
        "only one of the two configured headers was sent"
    );

    // Guards the loop below from being vacuous: if no metrics request ever
    // arrives, "every request was authenticated" is a statement about the traces
    // request alone and a regression in the metric exporter passes unnoticed.
    assert!(
        requests.iter().any(|r| r.url.path() == METRICS),
        "no /v1/metrics request arrived, so the metric exporter's headers are \
         untested. Paths seen: {:?}",
        requests.iter().map(|r| r.url.path()).collect::<Vec<_>>()
    );

    // Every OTLP request, whichever signal it carries, must be authenticated —
    // a collector requiring auth rejects the unauthenticated one, and losing
    // metrics silently is the same class of bug this fixes for traces.
    for r in &requests {
        assert!(
            r.headers.get("authorization").is_some(),
            "an OTLP request to {} was sent WITHOUT the configured headers; the \
             span and metric exporters are configured separately and this one \
             was missed",
            r.url.path()
        );
    }
}
