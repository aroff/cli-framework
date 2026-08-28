//! Proof that `traces_enabled: false` actually suppresses trace export (R20).
//!
//! The field was documented as "Whether to export trace spans" and then never
//! read — so an operator switching it off kept exporting every span, and the
//! only way to notice was to watch the collector. A test that asserted the
//! config field's value would have passed against that.
//!
//! So this asserts on **what the collector receives**: with traces off and
//! metrics on, `/v1/metrics` must still arrive and `/v1/traces` must not.
//! Checking both directions matters — an implementation that disabled the whole
//! pipeline would also produce "no traces" while silently taking metrics down
//! with it, which is a different bug wearing the same symptom.
//!
//! Spans are still *created* with traces disabled (only export is suppressed),
//! so W3C context still propagates to downstream services. That is deliberate:
//! a service opting out of its own trace storage should not sever the trace for
//! everyone downstream of it.
//!
//! # Why this is its own test binary
//!
//! `init_batch` installs a process-global subscriber and tracer provider.

use cli_framework::telemetry::TelemetryConfig;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

const TRACES: &str = "/v1/traces";
const METRICS: &str = "/v1/metrics";

#[tokio::test]
async fn traces_disabled_stops_spans_but_not_metrics() {
    let collector = MockServer::start().await;
    for p in [TRACES, METRICS] {
        Mock::given(method("POST"))
            .and(path(p))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(b""))
            .mount(&collector)
            .await;
    }

    let cfg = TelemetryConfig {
        endpoint: Some(collector.uri()),
        traces_enabled: false,
        metrics_enabled: true,
        ..Default::default()
    };

    let (telemetry, guard) =
        cli_framework::telemetry::init::init_batch(&cfg, "toggle-probe", "1.0")
            .expect("init must still succeed with traces off — metrics are still wanted");

    tracing::info_span!("probe.span").in_scope(|| {
        tracing::info!("probe");
    });
    telemetry.counter("probe.counter").add(1, &[]);

    guard.flush();
    tokio::time::sleep(std::time::Duration::from_millis(750)).await;

    let requests = collector.received_requests().await.unwrap_or_default();
    let paths: Vec<&str> = requests.iter().map(|r| r.url.path()).collect();

    assert!(
        paths.contains(&METRICS),
        "metrics stopped too. `traces_enabled: false` must suppress ONLY trace \
         export; taking the whole pipeline down is a different bug with the same \
         symptom. Collector saw: {paths:?}"
    );
    assert!(
        !paths.contains(&TRACES),
        "spans were exported with `traces_enabled: false` — the toggle is still \
         being ignored. Collector saw: {paths:?}"
    );
}
