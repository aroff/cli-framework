//! End-to-end proof that one trace spans two services (spec 017 R23/R24).
//!
//! This is the acceptance criterion spec 020 named for context propagation:
//! *two in-process services, assert a shared trace id*. It is deliberately a
//! full round trip rather than two unit tests, because propagation only has
//! value if all three links hold at once and each one fails silently on its own:
//!
//! 1. **Extract at A's edge** — the caller's `traceparent` becomes A's parent.
//! 2. **Inject on A → B** — A writes its own context onto the outbound request.
//! 3. **Extract at B's edge** — B continues the trace instead of rooting a new one.
//!
//! Break any single link and every service still produces well-formed spans and
//! a clean log line; the only visible symptom is that the trace backend shows
//! several unrelated traces instead of one. That is invisible to a test which
//! only asserts "a span was exported", which is why this asserts on the **trace
//! id bytes** and on **how many spans carry them**.
//!
//! # Why the assertion counts occurrences
//!
//! OTLP/HTTP protobuf is uncompressed by default and encodes `trace_id` as 16
//! raw bytes, so the caller's trace id is findable verbatim in the exported
//! body. Each `Span` message carries its own copy. So:
//!
//! - propagation fully working → A and B both carry the caller's id → **2 hits**
//! - inject or B's extract broken → only A carries it → **1 hit**
//! - A's extract broken → nobody carries it → **0 hits**
//!
//! Counting therefore distinguishes "propagated" from "coincidentally exported
//! a span", which a plain substring check cannot.
//!
//! # Why B has no `with_telemetry`
//!
//! `with_telemetry` installs a *process-global* subscriber and tracer provider.
//! A installs it; B's spans ride the same global provider and land in the same
//! collector. Giving B its own call would make the second one a silent no-op and
//! prove nothing. Same reason this is its own `[[test]]` binary.

use cli_framework::api::{ApiServerBuilder, ApiVersion, ApiVersionName, DefaultVersion, Stability};
use cli_framework::telemetry::propagation::TracedRequestBuilder as _;
use cli_framework::telemetry::TelemetryConfig;
use std::sync::OnceLock;
use std::time::Duration;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

const TRACES: &str = "/v1/traces";
const METRICS: &str = "/v1/metrics";

/// A fixed, valid W3C trace id. Using a constant rather than a random one keeps
/// the failure message actionable and lets the byte pattern be asserted exactly.
const CALLER_TRACE_ID: &str = "4bf92f3577b34da6a3ce929d0e0e4736";
const CALLER_SPAN_ID: &str = "00f067aa0ba902b7";

/// Where service A should forward. Set once, before A serves any traffic.
static SERVICE_B_URL: OnceLock<String> = OnceLock::new();

fn hex_to_bytes(s: &str) -> Vec<u8> {
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).expect("test constant is valid hex"))
        .collect()
}

fn count_occurrences(haystack: &[u8], needle: &[u8]) -> usize {
    if needle.is_empty() || haystack.len() < needle.len() {
        return 0;
    }
    haystack
        .windows(needle.len())
        .filter(|w| *w == needle)
        .count()
}

async fn find_free_port() -> u16 {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    drop(listener);
    port
}

async fn wait_until_ready(client: &reqwest::Client, addr: &str) -> bool {
    for _ in 0..200 {
        if let Ok(r) = client.get(format!("http://{addr}/healthz")).send().await {
            if r.status().is_success() {
                return true;
            }
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    false
}

/// Service A's handler: calls B, propagating the current trace context.
///
/// Returns B's outcome in the body so a downstream failure surfaces as a fixture
/// error in the test rather than quietly weakening the trace assertions.
async fn call_downstream() -> String {
    let url = SERVICE_B_URL
        .get()
        .expect("service B URL not set before serving");
    let client = reqwest::Client::new();
    match client.get(url).with_trace_context().send().await {
        Ok(r) if r.status().is_success() => "ok".to_string(),
        Ok(r) => format!("downstream status {}", r.status()),
        Err(e) => format!("downstream error {e}"),
    }
}

fn build_service(
    route: &str,
    handler: axum::routing::MethodRouter,
    telemetry: Option<(TelemetryConfig, &str)>,
) -> cli_framework::api::ApiServer {
    let router = axum::Router::new().route(route, handler);
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
        .default_version(DefaultVersion::Pinned(version));
    match telemetry {
        Some((cfg, name)) => builder.with_telemetry(cfg, name, "1.0.0").build(),
        None => builder.build(),
    }
}

#[tokio::test]
async fn one_trace_spans_two_services() {
    let collector = MockServer::start().await;
    for p in [TRACES, METRICS] {
        Mock::given(method("POST"))
            .and(path(p))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(b""))
            .mount(&collector)
            .await;
    }

    let port_a = find_free_port().await;
    let port_b = find_free_port().await;
    let addr_a = format!("127.0.0.1:{port_a}");
    let addr_b = format!("127.0.0.1:{port_b}");
    SERVICE_B_URL
        .set(format!("http://{addr_b}/api/v1/b"))
        .expect("URL set once");

    // A owns telemetry init for the whole process, so start it first and wait
    // for it to answer before B serves anything — otherwise B could handle the
    // forwarded request before a subscriber exists and export nothing.
    let api_a = build_service(
        "/a",
        axum::routing::get(call_downstream),
        Some((
            TelemetryConfig {
                endpoint: Some(collector.uri()),
                ..Default::default()
            },
            "svc-a",
        )),
    );
    let shutdown_a = api_a.shutdown_token();
    let serve_a = addr_a.clone();
    let handle_a = tokio::spawn(async move { api_a.serve(&serve_a).await });

    let client = reqwest::Client::new();
    assert!(
        wait_until_ready(&client, &addr_a).await,
        "service A never became ready on {addr_a}"
    );

    let api_b = build_service("/b", axum::routing::get(|| async { "ok" }), None);
    let shutdown_b = api_b.shutdown_token();
    let serve_b = addr_b.clone();
    let handle_b = tokio::spawn(async move { api_b.serve(&serve_b).await });
    assert!(
        wait_until_ready(&client, &addr_b).await,
        "service B never became ready on {addr_b}"
    );

    // The inbound edge: a caller that is already in a trace.
    let resp = client
        .get(format!("http://{addr_a}/api/v1/a"))
        .header(
            "traceparent",
            format!("00-{CALLER_TRACE_ID}-{CALLER_SPAN_ID}-01"),
        )
        .send()
        .await
        .expect("request to A should succeed at the transport level");
    assert!(
        resp.status().is_success(),
        "fixture is wrong, not the telemetry: A returned {}",
        resp.status()
    );
    assert_eq!(
        resp.text().await.unwrap(),
        "ok",
        "A could not reach B, so nothing was propagated and the trace \
         assertions below would be vacuous"
    );

    // Shut A down so its guard drops and force-flushes; B's spans went to the
    // same global provider, so they flush with it.
    shutdown_a.cancel();
    shutdown_b.cancel();
    let _ = tokio::time::timeout(Duration::from_secs(10), handle_a).await;
    let _ = tokio::time::timeout(Duration::from_secs(10), handle_b).await;
    tokio::time::sleep(Duration::from_millis(750)).await;

    let requests = collector.received_requests().await.unwrap_or_default();
    let trace_bodies: Vec<&[u8]> = requests
        .iter()
        .filter(|r| r.url.path() == TRACES)
        .map(|r| r.body.as_slice())
        .collect();
    assert!(
        !trace_bodies.is_empty(),
        "no spans were exported at all. Collector saw: {:?}",
        requests.iter().map(|r| r.url.path()).collect::<Vec<_>>()
    );

    let has = |needle: &[u8]| {
        trace_bodies
            .iter()
            .any(|b| count_occurrences(b, needle) > 0)
    };

    // Both services must have produced a span, or the count below could reach 2
    // from a single service exporting twice.
    assert!(
        has(b"GET /api/v1/a"),
        "service A exported no span for the request it served"
    );
    assert!(
        has(b"GET /api/v1/b"),
        "service B exported no span — A's outbound call was never traced, so \
         `with_trace_context` is not reaching B"
    );

    let trace_id = hex_to_bytes(CALLER_TRACE_ID);
    let hits: usize = trace_bodies
        .iter()
        .map(|b| count_occurrences(b, &trace_id))
        .sum();

    assert!(
        hits >= 2,
        "expected the caller's trace id on both services' spans, found it on \
         {hits}. 0 = service A ignored the inbound `traceparent` and started its \
         own root trace; 1 = A joined the caller's trace but B did not, so \
         either injection on A's outbound call or extraction at B's edge is \
         broken. Either way the platform shows disconnected traces per service, \
         which is exactly the state this test exists to prevent."
    );
}
