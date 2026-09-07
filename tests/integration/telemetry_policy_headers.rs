//! Proof that a credential carried on a resolved `TelemetryPolicy` reaches the
//! collector (PRD 025 export boundary).
//!
//! # Why `telemetry_otlp_headers.rs` does not cover this
//!
//! That file drives `init_batch` -- the **config** path, the one the deprecated
//! `with_telemetry` shim takes. A spec 025 app takes a different one:
//! `run_startup` resolves a `TelemetryPolicy` and calls `init_from_policy`,
//! whose exporters come from `span_exporter_for_policy` and
//! `metric_exporter_for_policy`. Those are separate builder call sites with
//! their own `.with_headers(...)`, and for a while they had none: a policy
//! carrying an `authorization` header exported without it, and every
//! authenticated collector answered 401 somewhere nobody was looking. Two
//! export paths need two proofs.
//!
//! # Why this test deliberately does NOT set the env var
//!
//! `opentelemetry-otlp` reads `OTEL_EXPORTER_OTLP_HEADERS` itself. With it set,
//! the SDK supplies the header and this test passes against an exporter that
//! was handed nothing -- exactly the vacuity `telemetry_otlp_headers.rs`
//! records having shipped once already. It is removed below, so the only
//! possible source of the header on the wire is `TelemetryPolicy::headers`.
//!
//! # Why the span is stamped by hand instead of running an `AppBuilder`
//!
//! `redact_span` drops every span whose `cli.probe` attribute is absent, and no
//! production instrumentation site records that attribute yet: the dispatch
//! root span in `src/app/builder.rs` declares `cli.probe` as
//! `tracing::field::Empty` and nothing fills it in. So an `AppBuilder` run
//! against this collector POSTs no trace request at all, and the traces half of
//! this test would be an assertion about an empty collector. Wiring the probes
//! is its own work item; until it lands, the span below carries the attribute
//! directly so that a request actually arrives for its headers to be read off.

use cli_framework::telemetry::probes::metrics;
use cli_framework::telemetry::{
    init_from_policy, resolve_policy, Attribution, Deployment, ProbeRegistry, ServiceIdentity,
    TelemetryInputs,
};
use opentelemetry::trace::{Span, Tracer};
use opentelemetry::KeyValue;
use secrecy::SecretString;
use std::sync::Arc;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

const TRACES: &str = "/v1/traces";
const METRICS: &str = "/v1/metrics";

#[tokio::test]
async fn policy_headers_are_sent_to_the_collector() {
    let collector = MockServer::start().await;
    for p in [TRACES, METRICS] {
        Mock::given(method("POST"))
            .and(path(p))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(b""))
            .mount(&collector)
            .await;
    }

    // The load-bearing line: with this set the SDK supplies the header on its
    // own and this test passes against a broken exporter config.
    // SAFETY: single-threaded test setup, before any task is spawned.
    unsafe {
        std::env::remove_var("OTEL_EXPORTER_OTLP_HEADERS");
    }

    let policy = Arc::new(resolve_policy(TelemetryInputs {
        app: "policyheaders".to_string(),
        deployment: Deployment::Service,
        endpoint: Some(collector.uri()),
        attribution: Attribution::Pseudonymous,
        install_id: Some("install-policy-headers".to_string()),
        session_id: "session-policy-headers".to_string(),
        registry: ProbeRegistry::with_builtins(),
        store_available: true,
        // Explicit rather than leaning on `resolve_policy` normalising 0.0 to
        // 1.0: a sampler that drops the single span this test emits would leave
        // the traces assertion below with nothing to read.
        sample_ratio: 1.0,
        headers: Some(SecretString::from(
            "authorization=Bearer tok-policy,x-scope-orgid=acme".to_string(),
        )),
        ..Default::default()
    }));

    // A `Service` deployment with an endpoint resolves to `diagnostic`, above
    // `cli.command`'s `usage` minimum. Asserted rather than assumed: a policy
    // that does not export builds no exporter at all, and every assertion below
    // would then be a statement about an empty collector.
    assert!(
        policy.exports(),
        "the resolved policy does not export, so nothing below proves anything"
    );

    let (handle, guard) = init_from_policy(
        policy,
        ServiceIdentity {
            name: "policyheaders".to_string(),
            version: "1.0.0".to_string(),
        },
    )
    .expect("a Service policy with an endpoint must build exporters");

    // `cli.probe` stamped by hand -- see the module doc. Without it the export
    // boundary drops this span and no /v1/traces request is ever made.
    let tracer = guard.tracer("policy-headers-test");
    let mut span = tracer
        .span_builder("cli.command")
        .with_attributes(vec![
            KeyValue::new("cli.probe", "cli.command"),
            KeyValue::new("command", "probe"),
        ])
        .start(&tracer);
    span.end();

    // Both signals, on purpose. `span_exporter_for_policy` and
    // `metric_exporter_for_policy` are separate builders with separate
    // `.with_headers(...)` calls, so a fix applied to one and missed on the
    // other has to be catchable -- and it is not unless a metric is actually
    // recorded here. `force_flush` on a meter provider with nothing in it POSTs
    // nothing.
    handle
        .counter(metrics::COMMAND_INVOCATIONS)
        .add(1, &[KeyValue::new("command", "probe")]);

    guard.flush();
    tokio::time::sleep(std::time::Duration::from_millis(750)).await;

    let requests = collector.received_requests().await.unwrap_or_default();
    assert!(
        !requests.is_empty(),
        "collector received nothing, so this proves nothing about headers"
    );

    // Pinned to /v1/traces specifically. Accepting "any request that carried
    // the header" would let a regression in the SPAN exporter pass whenever the
    // metrics exporter happened to flush first.
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
             `TelemetryPolicy::headers` was populated but never handed to the \
             span exporter builder, so a spec 025 app that resolves a \
             credential exports to an authenticated collector without one."
        )
    });
    assert_eq!(auth, "Bearer tok-policy");
    assert_eq!(
        req.headers
            .get("x-scope-orgid")
            .map(|v| v.to_str().unwrap()),
        Some("acme"),
        "only one of the two configured headers was sent"
    );

    // Guards the loop below from being vacuous: if no metrics request ever
    // arrives, "every request was authenticated" is a statement about the
    // traces request alone and a regression in `metric_exporter_for_policy`
    // passes unnoticed.
    assert!(
        requests.iter().any(|r| r.url.path() == METRICS),
        "no /v1/metrics request arrived, so the metric exporter's headers are \
         untested. Paths seen: {:?}",
        requests.iter().map(|r| r.url.path()).collect::<Vec<_>>()
    );

    for r in &requests {
        assert!(
            r.headers.get("authorization").is_some(),
            "an OTLP request to {} was sent WITHOUT the policy's headers; the \
             span and metric exporters are configured separately and this one \
             was missed",
            r.url.path()
        );
    }
}
