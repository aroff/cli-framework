//! Tier 2: Wiremock OTLP transport test — proves opentelemetry-otlp serialises
//! and POSTs spans to the correct endpoint over HTTP/protobuf.

use cli_framework::telemetry::{init, TelemetryConfig};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

// Tests the OTLP HTTP transport using init_batch (BatchSpanProcessor) which is
// the correct processor for async environments (servers, tests with async runtimes).
// init_simple uses SimpleSpanProcessor which exports synchronously on span-end via
// reqwest::blocking, creating a nested Tokio runtime — this is intended for
// single-threaded CLI processes, not async contexts.  See unit/telemetry.rs for
// init_simple coverage via TestExporter.
#[tokio::test]
async fn otlp_http_transport_posts_to_v1_traces() {
    let mock_server = MockServer::start().await;

    Mock::given(method("POST"))
        .and(path("/v1/traces"))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(b""))
        .mount(&mock_server)
        .await;

    let cfg = TelemetryConfig {
        endpoint: Some(mock_server.uri()),
        ..Default::default()
    };

    let (_handle, guard) = match init::init_batch(&cfg, "test-svc", "0.1") {
        Some(pair) => pair,
        None => panic!("init_batch returned None with a valid endpoint"),
    };

    use tracing_subscriber::prelude::*;
    // Use guard.tracer() instead of the global to avoid races with parallel tests
    let otel_layer = tracing_opentelemetry::layer().with_tracer(guard.tracer("cli-framework"));
    let subscriber = tracing_subscriber::registry().with(otel_layer);

    tracing::subscriber::with_default(subscriber, || {
        let _span = tracing::info_span!("otlp.transport.test").entered();
    });

    guard.flush();
    tokio::time::sleep(std::time::Duration::from_millis(300)).await;

    let requests = mock_server.received_requests().await.unwrap_or_default();
    let traces_hit = requests.iter().any(|r| r.url.path() == "/v1/traces");
    assert!(
        traces_hit,
        "expected POST /v1/traces, got: {:?}",
        requests
            .iter()
            .map(|r| format!("{} {}", r.method, r.url.path()))
            .collect::<Vec<_>>()
    );

    if let Some(req) = requests.iter().find(|r| r.url.path() == "/v1/traces") {
        let ct = req
            .headers
            .get("content-type")
            .map(|v| v.to_str().unwrap_or(""))
            .unwrap_or("");
        assert!(
            ct.contains("protobuf"),
            "expected protobuf content-type, got: {}",
            ct
        );
    }
}
