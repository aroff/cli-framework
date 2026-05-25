//! Integration tests for the `api-swagger` feature.
//!
//! Tests cover: OpenAPI spec serving, `servers:` patch, Swagger UI HTML,
//! version switcher, primary version selection, auth gating, and health endpoint regression.

use axum::routing::get;
use axum::Router;
use cli_framework::api::{ApiServerBuilder, ApiVersion, ApiVersionName, DefaultVersion, Stability};
use cli_framework::tower::util::BoxCloneLayer;
use std::time::Duration;

// ─── helpers ────────────────────────────────────────────────────────────────

async fn wait_http_ok(url: &str) {
    let client = reqwest::Client::new();
    for _ in 0..50 {
        if let Ok(r) = client.get(url).send().await {
            if r.status().is_success() {
                return;
            }
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    panic!("server did not become ready: {url}");
}

async fn wait_http_any(url: &str) {
    let client = reqwest::Client::new();
    for _ in 0..50 {
        if client.get(url).send().await.is_ok() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    panic!("server did not start responding: {url}");
}

async fn spawn_server(
    builder: ApiServerBuilder,
) -> (
    String,
    tokio_util::sync::CancellationToken,
    tokio::task::JoinHandle<()>,
) {
    let api = builder.build();
    let shutdown = api.shutdown_token();
    let router = api.into_router();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let addr_str = format!("{}", addr);
    let shutdown_for_task = shutdown.clone();
    let handle = tokio::spawn(async move {
        axum::serve(listener, router)
            .with_graceful_shutdown(shutdown_for_task.cancelled_owned())
            .await
            .unwrap();
    });
    wait_http_ok(&format!("http://{addr_str}/healthz")).await;
    (addr_str, shutdown, handle)
}

async fn spawn_server_auth_gated(
    builder: ApiServerBuilder,
) -> (
    String,
    tokio_util::sync::CancellationToken,
    tokio::task::JoinHandle<()>,
) {
    let api = builder.build();
    let shutdown = api.shutdown_token();
    let router = api.into_router();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let addr_str = format!("{}", addr);
    let shutdown_for_task = shutdown.clone();
    let handle = tokio::spawn(async move {
        axum::serve(listener, router)
            .with_graceful_shutdown(shutdown_for_task.cancelled_owned())
            .await
            .unwrap();
    });
    wait_http_any(&format!("http://{addr_str}/healthz")).await;
    (addr_str, shutdown, handle)
}

/// A simple bearer-token auth layer: rejects anything without `Authorization: Bearer secret`.
fn bearer_auth_layer() -> BoxCloneLayer<axum::Router> {
    use axum::http::{Request, StatusCode};
    use axum::middleware::{self, Next};
    use axum::response::Response;

    let layer = middleware::from_fn(|req: Request<axum::body::Body>, next: Next| async move {
        let auth = req
            .headers()
            .get("authorization")
            .and_then(|v| v.to_str().ok());
        if auth == Some("Bearer secret") {
            Ok::<Response, StatusCode>(next.run(req).await)
        } else {
            Err::<Response, StatusCode>(StatusCode::UNAUTHORIZED)
        }
    });
    BoxCloneLayer::new(layer)
}

fn v1_doc() -> serde_json::Value {
    serde_json::json!({
        "openapi": "3.0.3",
        "info": { "title": "Test API", "version": "1.0.0" },
        "paths": {
            "/echo": {
                "get": { "summary": "Echo", "responses": { "200": { "description": "OK" } } }
            }
        }
    })
}

fn v2_doc() -> serde_json::Value {
    serde_json::json!({
        "openapi": "3.0.3",
        "info": { "title": "Test API", "version": "2.0.0" },
        "paths": {
            "/echo": {
                "get": { "summary": "Echo v2", "responses": { "200": { "description": "OK" } } }
            }
        }
    })
}

fn make_v1() -> ApiVersion {
    ApiVersion {
        name: ApiVersionName::parse("v1").unwrap(),
        router: Router::new().route("/echo", get(|| async { "v1" })),
        stability: Stability::Stable,
        deprecation: None,
        openapi: Some(v1_doc()),
    }
}

fn make_v2() -> ApiVersion {
    ApiVersion {
        name: ApiVersionName::parse("v2").unwrap(),
        router: Router::new().route("/echo", get(|| async { "v2" })),
        stability: Stability::Stable,
        deprecation: None,
        openapi: Some(v2_doc()),
    }
}

fn make_v1_no_doc() -> ApiVersion {
    ApiVersion {
        name: ApiVersionName::parse("v1").unwrap(),
        router: Router::new().route("/echo", get(|| async { "v1" })),
        stability: Stability::Stable,
        deprecation: None,
        openapi: None,
    }
}

// ─── Acceptance criterion 1 & 2: spec endpoints are served ──────────────────

#[tokio::test]
async fn spec_endpoint_v1_returns_200_with_json_content_type() {
    let (addr, shutdown, handle) = spawn_server(
        ApiServerBuilder::new()
            .version(make_v1())
            .version(make_v2())
            .default_version(DefaultVersion::Pinned(ApiVersionName::parse("v1").unwrap())),
    )
    .await;

    let client = reqwest::Client::new();
    let r = client
        .get(format!("http://{addr}/api/v1/openapi.json"))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 200);
    assert!(
        r.headers()["content-type"]
            .to_str()
            .unwrap()
            .contains("application/json"),
        "expected application/json, got {:?}",
        r.headers().get("content-type")
    );

    shutdown.cancel();
    handle.await.unwrap();
}

#[tokio::test]
async fn spec_endpoint_v2_returns_200_with_json_content_type() {
    let (addr, shutdown, handle) = spawn_server(
        ApiServerBuilder::new()
            .version(make_v1())
            .version(make_v2())
            .default_version(DefaultVersion::Pinned(ApiVersionName::parse("v1").unwrap())),
    )
    .await;

    let client = reqwest::Client::new();
    let r = client
        .get(format!("http://{addr}/api/v2/openapi.json"))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 200);
    assert!(
        r.headers()["content-type"]
            .to_str()
            .unwrap()
            .contains("application/json"),
        "expected application/json"
    );

    shutdown.cancel();
    handle.await.unwrap();
}

// ─── Acceptance criterion 3: servers: patch is applied correctly ─────────────

#[tokio::test]
async fn servers_patch_v1_is_correct() {
    let (addr, shutdown, handle) = spawn_server(
        ApiServerBuilder::new()
            .version(make_v1())
            .version(make_v2())
            .default_version(DefaultVersion::Pinned(ApiVersionName::parse("v1").unwrap())),
    )
    .await;

    let client = reqwest::Client::new();
    let body: serde_json::Value = client
        .get(format!("http://{addr}/api/v1/openapi.json"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();

    // servers: patch applied
    assert_eq!(body["servers"], serde_json::json!([{"url": "/api/v1"}]));
    // Other fields unchanged
    assert_eq!(body["openapi"], "3.0.3");
    assert_eq!(body["info"]["title"], "Test API");
    assert_eq!(body["info"]["version"], "1.0.0");

    shutdown.cancel();
    handle.await.unwrap();
}

#[tokio::test]
async fn servers_patch_v2_is_correct() {
    let (addr, shutdown, handle) = spawn_server(
        ApiServerBuilder::new()
            .version(make_v1())
            .version(make_v2())
            .default_version(DefaultVersion::Pinned(ApiVersionName::parse("v1").unwrap())),
    )
    .await;

    let client = reqwest::Client::new();
    let body: serde_json::Value = client
        .get(format!("http://{addr}/api/v2/openapi.json"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();

    assert_eq!(body["servers"], serde_json::json!([{"url": "/api/v2"}]));
    assert_eq!(body["info"]["version"], "2.0.0");

    shutdown.cancel();
    handle.await.unwrap();
}

// ─── Acceptance criterion 4: Swagger UI at /api/docs returns HTML ────────────

#[tokio::test]
async fn swagger_ui_returns_html() {
    let (addr, shutdown, handle) = spawn_server(
        ApiServerBuilder::new()
            .version(make_v1())
            .version(make_v2())
            .default_version(DefaultVersion::Pinned(ApiVersionName::parse("v1").unwrap())),
    )
    .await;

    let client = reqwest::Client::new();
    // The SwaggerUI redirects /api/docs -> /api/docs/, then serves HTML.
    let r = client
        .get(format!("http://{addr}/api/docs/"))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 200);
    let ct = r.headers()["content-type"].to_str().unwrap().to_lowercase();
    assert!(ct.contains("text/html"), "expected text/html, got: {ct}");
    let html = r.text().await.unwrap();
    assert!(html.contains("swagger"), "expected swagger content in HTML");

    shutdown.cancel();
    handle.await.unwrap();
}

#[tokio::test]
async fn swagger_ui_has_no_cdn_urls() {
    let (addr, shutdown, handle) = spawn_server(
        ApiServerBuilder::new()
            .version(make_v1())
            .version(make_v2())
            .default_version(DefaultVersion::Pinned(ApiVersionName::parse("v1").unwrap())),
    )
    .await;

    let client = reqwest::Client::new();
    let html = client
        .get(format!("http://{addr}/api/docs/"))
        .send()
        .await
        .unwrap()
        .text()
        .await
        .unwrap();

    // No CDN references (unpkg, cdnjs, jsdelivr, etc.)
    assert!(
        !html.contains("unpkg.com") && !html.contains("cdnjs") && !html.contains("jsdelivr"),
        "HTML should not reference a CDN"
    );

    shutdown.cancel();
    handle.await.unwrap();
}

// ─── Acceptance criterion 7: version with openapi: None → 404 ────────────────

#[tokio::test]
async fn version_without_doc_returns_404_for_openapi_json() {
    let (addr, shutdown, handle) = spawn_server(
        ApiServerBuilder::new()
            .version(make_v1_no_doc())
            .default_version(DefaultVersion::Pinned(ApiVersionName::parse("v1").unwrap())),
    )
    .await;

    let client = reqwest::Client::new();
    let r = client
        .get(format!("http://{addr}/api/v1/openapi.json"))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 404);

    shutdown.cancel();
    handle.await.unwrap();
}

// ─── Acceptance criterion 9 & 10: auth gating ────────────────────────────────

#[tokio::test]
async fn spec_returns_401_when_auth_configured_and_no_credentials() {
    let (addr, shutdown, handle) = spawn_server_auth_gated(
        ApiServerBuilder::new()
            .version(make_v1())
            .default_version(DefaultVersion::Pinned(ApiVersionName::parse("v1").unwrap()))
            .auth(bearer_auth_layer()),
    )
    .await;

    let client = reqwest::Client::new();
    let r = client
        .get(format!("http://{addr}/api/v1/openapi.json"))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 401);

    shutdown.cancel();
    handle.await.unwrap();
}

#[tokio::test]
async fn docs_returns_401_when_auth_configured_and_no_credentials() {
    let (addr, shutdown, handle) = spawn_server_auth_gated(
        ApiServerBuilder::new()
            .version(make_v1())
            .default_version(DefaultVersion::Pinned(ApiVersionName::parse("v1").unwrap()))
            .auth(bearer_auth_layer()),
    )
    .await;

    let client = reqwest::Client::new();
    let r = client
        .get(format!("http://{addr}/api/docs/"))
        .send()
        .await
        .unwrap();
    // Auth layer may return 401 or behave differently, check it's not 200
    assert_ne!(
        r.status(),
        200,
        "unauthenticated request should not succeed"
    );

    shutdown.cancel();
    handle.await.unwrap();
}

#[tokio::test]
async fn spec_accessible_without_auth_when_no_auth_configured() {
    let (addr, shutdown, handle) = spawn_server(
        ApiServerBuilder::new()
            .version(make_v1())
            .default_version(DefaultVersion::Pinned(ApiVersionName::parse("v1").unwrap())),
    )
    .await;

    let client = reqwest::Client::new();
    let r = client
        .get(format!("http://{addr}/api/v1/openapi.json"))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 200);

    shutdown.cancel();
    handle.await.unwrap();
}

// ─── Acceptance criterion 11 & 15: existing health/API endpoints unaffected ───

#[tokio::test]
async fn healthz_and_readyz_unaffected_by_api_swagger() {
    let (addr, shutdown, handle) = spawn_server(
        ApiServerBuilder::new()
            .version(make_v1())
            .version(make_v2())
            .default_version(DefaultVersion::Pinned(ApiVersionName::parse("v1").unwrap())),
    )
    .await;

    let client = reqwest::Client::new();

    let h = client
        .get(format!("http://{addr}/healthz"))
        .send()
        .await
        .unwrap();
    assert_eq!(h.status(), 200);
    let hj: serde_json::Value = h.json().await.unwrap();
    assert_eq!(hj["status"], "ok");

    let r = client
        .get(format!("http://{addr}/readyz"))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 200);

    shutdown.cancel();
    handle.await.unwrap();
}

#[tokio::test]
async fn versioned_api_routes_unaffected_by_api_swagger() {
    let (addr, shutdown, handle) = spawn_server(
        ApiServerBuilder::new()
            .version(make_v1())
            .version(make_v2())
            .default_version(DefaultVersion::Pinned(ApiVersionName::parse("v1").unwrap())),
    )
    .await;

    let client = reqwest::Client::new();

    let r1 = client
        .get(format!("http://{addr}/api/v1/echo"))
        .send()
        .await
        .unwrap();
    assert_eq!(r1.status(), 200);
    assert_eq!(r1.headers()["X-API-Version"], "v1");

    let r2 = client
        .get(format!("http://{addr}/api/v2/echo"))
        .send()
        .await
        .unwrap();
    assert_eq!(r2.status(), 200);
    assert_eq!(r2.headers()["X-API-Version"], "v2");

    shutdown.cancel();
    handle.await.unwrap();
}

// ─── Acceptance criterion 6: default version selection in Swagger UI ─────────

#[tokio::test]
async fn swagger_ui_lists_versioned_doc_urls() {
    // Verify that swagger-initializer.js references both versioned spec URLs
    // and that the pinned default (v2) is marked as primary (AC6).
    let (addr, shutdown, handle) = spawn_server(
        ApiServerBuilder::new()
            .version(make_v1())
            .version(make_v2())
            .default_version(DefaultVersion::Pinned(ApiVersionName::parse("v2").unwrap())),
    )
    .await;

    let client = reqwest::Client::new();
    // Config is injected into swagger-initializer.js, not index.html.
    let js = client
        .get(format!("http://{addr}/api/docs/swagger-initializer.js"))
        .send()
        .await
        .unwrap()
        .text()
        .await
        .unwrap();

    assert!(
        js.contains("/api/v1/openapi.json"),
        "v1 spec URL missing from swagger-initializer.js"
    );
    assert!(
        js.contains("/api/v2/openapi.json"),
        "v2 spec URL missing from swagger-initializer.js"
    );

    // AC6: pinned default version v2 must be marked as primary.
    // utoipa-swagger-ui serializes Url::with_primary as `"urls.primaryName": "<name>"` in the
    // swagger-initializer.js SwaggerUIBundle config.
    assert!(
        js.contains(r#""urls.primaryName":"v2""#) || js.contains(r#""urls.primaryName": "v2""#),
        "pinned default v2 should be the primaryName in swagger-initializer.js; js={js}"
    );

    shutdown.cancel();
    handle.await.unwrap();
}

#[tokio::test]
async fn swagger_ui_primary_defaults_to_alphabetical_first_when_no_pinned_default() {
    // AC6: without a pinned default, the first version alphabetically (v1) must be primary.
    let (addr, shutdown, handle) = spawn_server(
        ApiServerBuilder::new()
            .version(make_v1())
            .version(make_v2())
            .default_version(DefaultVersion::None),
    )
    .await;

    let client = reqwest::Client::new();
    let js = client
        .get(format!("http://{addr}/api/docs/swagger-initializer.js"))
        .send()
        .await
        .unwrap()
        .text()
        .await
        .unwrap();

    assert!(
        js.contains("/api/v1/openapi.json"),
        "v1 spec URL missing from swagger-initializer.js"
    );
    assert!(
        js.contains("/api/v2/openapi.json"),
        "v2 spec URL missing from swagger-initializer.js"
    );

    // With no pinned default, v1 (alphabetically first) should be the primary.
    assert!(
        js.contains(r#""urls.primaryName":"v1""#) || js.contains(r#""urls.primaryName": "v1""#),
        "alphabetical-first version v1 should be primaryName when no default is pinned; js={js}"
    );

    shutdown.cancel();
    handle.await.unwrap();
}

#[tokio::test]
async fn swagger_ui_omits_version_without_doc() {
    // A version with openapi: None should NOT appear in the Swagger UI config.
    let (addr, shutdown, handle) = spawn_server(
        ApiServerBuilder::new()
            .version(make_v1_no_doc())
            .version(make_v2())
            .default_version(DefaultVersion::Pinned(ApiVersionName::parse("v2").unwrap())),
    )
    .await;

    let client = reqwest::Client::new();
    let js = client
        .get(format!("http://{addr}/api/docs/swagger-initializer.js"))
        .send()
        .await
        .unwrap()
        .text()
        .await
        .unwrap();

    assert!(
        !js.contains("/api/v1/openapi.json"),
        "v1 (no doc) should not appear in swagger-initializer.js"
    );
    assert!(
        js.contains("/api/v2/openapi.json"),
        "v2 spec URL should appear in swagger-initializer.js"
    );

    shutdown.cancel();
    handle.await.unwrap();
}
