//! End-to-end proof that an HTTP request to an `ApiServer` exports a span.
//!
//! [`telemetry_end_to_end`] covers the CLI dispatch path (`AppBuilder` →
//! `run_with_args` → `cli.command`). This covers the server path, which was a
//! separate blind spot with the same shape and a worse blast radius: every
//! consumer serving HTTP exported **nothing** per request.
//!
//! The cause was that `with_tracing` — the layer wrapping every versioned
//! router, mount, `/mcp` and the root fallback — emitted only a
//! `tracing::info!` **event**. `tracing-opentelemetry` maps an event onto its
//! *enclosing* span, so an event with no enclosing span is discarded outright.
//! Telemetry could be fully configured and the bridge correctly installed, and
//! there was still nothing for it to carry. A log line went out every time,
//! which is exactly why nobody noticed.
//!
//! The rule for anything added here: touch only public API a consumer would
//! touch (`ApiServerBuilder` → `with_telemetry` → `serve`). Never construct a
//! subscriber, a layer, or a provider directly — doing so re-creates the blind
//! spot instead of catching it.
//!
//! # Why this is one test and not several
//!
//! `with_telemetry` installs a *process-global* `tracing` subscriber bound to
//! the first provider that wins the race. A second test in this binary would
//! export into the first test's collector and assert against an empty one.

use cli_framework::api::{ApiServerBuilder, ApiVersion, ApiVersionName, DefaultVersion, Stability};
use cli_framework::telemetry::TelemetryConfig;
use std::time::Duration;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

/// Paths the OTLP/HTTP exporters POST to, relative to the configured endpoint.
const TRACES: &str = "/v1/traces";
const METRICS: &str = "/v1/metrics";

async fn find_free_port() -> u16 {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    drop(listener);
    port
}

#[tokio::test]
async fn api_request_exports_a_server_span() {
    // Stands in for an OTel collector. Metrics is mocked too: the meter
    // provider's periodic reader posts there regardless, and leaving it
    // unmounted buries the interesting failure under exporter errors.
    let collector = MockServer::start().await;
    for p in [TRACES, METRICS] {
        Mock::given(method("POST"))
            .and(path(p))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(b""))
            .mount(&collector)
            .await;
    }

    // A route with a PATH PARAMETER on purpose: the span must be named from the
    // matched pattern (`/items/{id}`), never the concrete path (`/items/42`),
    // or every distinct id becomes its own operation name in the backend.
    let router = axum::Router::new().route("/items/{id}", axum::routing::get(|| async { "ok" }));

    let version = ApiVersionName::parse("v1").unwrap();
    let builder = ApiServerBuilder::new()
        .version(ApiVersion {
            name: version.clone(),
            router,
            stability: Stability::Stable,
            deprecation: None,
            #[cfg(feature = "api-swagger")]
            openapi: None,
        })
        .default_version(DefaultVersion::Pinned(version))
        .with_telemetry(
            TelemetryConfig {
                endpoint: Some(collector.uri()),
                ..Default::default()
            },
            "probe-api",
            "1.2.3",
        );

    let port = find_free_port().await;
    let addr = format!("127.0.0.1:{port}");
    let api = builder.build();
    let shutdown = api.shutdown_token();
    let serve_addr = addr.clone();
    let handle = tokio::spawn(async move { api.serve(&serve_addr).await });

    let client = reqwest::Client::new();
    let mut ready = false;
    for _ in 0..100 {
        if let Ok(r) = client.get(format!("http://{addr}/healthz")).send().await {
            if r.status().is_success() {
                ready = true;
                break;
            }
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert!(ready, "server never became ready on {addr}");

    let resp = client
        .get(format!("http://{addr}/api/v1/items/42"))
        .send()
        .await
        .expect("request should succeed at the transport level");
    assert!(
        resp.status().is_success(),
        "fixture is wrong, not the telemetry: got {}",
        resp.status()
    );

    // Shut down so `serve`'s TelemetryGuard drops and force-flushes; the batch
    // worker still needs a beat to land the POST.
    shutdown.cancel();
    let _ = tokio::time::timeout(Duration::from_secs(10), handle).await;
    tokio::time::sleep(Duration::from_millis(750)).await;

    let requests = collector.received_requests().await.unwrap_or_default();
    let paths: Vec<&str> = requests.iter().map(|r| r.url.path()).collect();

    assert!(
        paths.contains(&TRACES),
        "an API request exported no spans — `with_tracing` is emitting an event \
         with no enclosing span, so the OTel bridge has nothing to carry. \
         Collector saw: {paths:?}"
    );

    // OTLP/HTTP protobuf stores strings verbatim and is uncompressed by
    // default, so the span name survives as a readable substring.
    let bodies: Vec<&[u8]> = requests
        .iter()
        .filter(|r| r.url.path() == TRACES)
        .map(|r| r.body.as_slice())
        .collect();
    let has = |needle: &[u8]| {
        bodies
            .iter()
            .any(|b| b.windows(needle.len()).any(|w| w == needle))
    };

    assert!(
        has(b"GET /api/v1/items/{id}"),
        "spans were exported but none is named from the matched route pattern"
    );
    // The guard against cardinality blowup: the concrete id must not appear as
    // part of the span name. `/items/42` as a name would mint one operation per
    // id and make the backend unusable.
    assert!(
        !has(b"GET /api/v1/items/42"),
        "span is named from the CONCRETE path — every distinct id becomes its \
         own operation name in the trace backend"
    );
}
