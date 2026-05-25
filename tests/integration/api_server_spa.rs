use axum::response::IntoResponse;
use axum::routing::get;
use axum::Router;
use cli_framework::api::{ApiServerBuilder, ApiVersion, ApiVersionName, Stability};
use cli_framework::tower::util::BoxCloneLayer;
use std::time::Duration;

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

async fn spawn_inline_server(
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

fn v1_router() -> Router {
    Router::new().route(
        "/echo",
        get(|| async { axum::Json(serde_json::json!({"version": "v1"})) }),
    )
}

fn spa_router() -> Router {
    Router::new().fallback(|| async { "spa" })
}

fn base_builder() -> ApiServerBuilder {
    ApiServerBuilder::new().version(ApiVersion {
        name: ApiVersionName::parse("v1").unwrap(),
        router: v1_router(),
        stability: Stability::Stable,
        deprecation: None,
        #[cfg(feature = "api-swagger")]
        openapi: None,
    })
}

#[tokio::test]
async fn fallback_serves_root() {
    let (addr, shutdown, handle) =
        spawn_inline_server(base_builder().root_fallback(spa_router())).await;

    let resp = reqwest::get(format!("http://{addr}/")).await.unwrap();
    assert_eq!(resp.status(), 200);
    assert!(resp.text().await.unwrap().contains("spa"));

    shutdown.cancel();
    handle.await.unwrap();
}

#[tokio::test]
async fn fallback_serves_asset_path() {
    let (addr, shutdown, handle) =
        spawn_inline_server(base_builder().root_fallback(spa_router())).await;

    let resp = reqwest::get(format!("http://{addr}/assets/app.js"))
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    assert!(resp.text().await.unwrap().contains("spa"));

    shutdown.cancel();
    handle.await.unwrap();
}

#[tokio::test]
async fn fallback_serves_deep_link() {
    let (addr, shutdown, handle) =
        spawn_inline_server(base_builder().root_fallback(spa_router())).await;

    let resp = reqwest::get(format!("http://{addr}/spa/deep/link"))
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    assert!(resp.text().await.unwrap().contains("spa"));

    shutdown.cancel();
    handle.await.unwrap();
}

#[tokio::test]
async fn framework_routes_win_over_fallback() {
    let mount_router = Router::new().route(
        "/",
        get(|| async { axum::Json(serde_json::json!({"mount": true})) }),
    );
    let (addr, shutdown, handle) = spawn_inline_server(
        base_builder()
            .mount("/status", mount_router)
            .root_fallback(spa_router()),
    )
    .await;

    let client = reqwest::Client::new();

    // Versioned API route wins.
    let r = client
        .get(format!("http://{addr}/api/v1/echo"))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 200);
    let body: serde_json::Value = r.json().await.unwrap();
    assert_eq!(body["version"], "v1");

    // Unversioned /api/ handler wins.
    let r = client
        .get(format!("http://{addr}/api/"))
        .send()
        .await
        .unwrap();
    assert!(r.status().as_u16() < 500, "expected non-5xx for /api/");
    let body_text = r.text().await.unwrap();
    assert!(
        !body_text.contains("spa"),
        "/api/ should not be the fallback"
    );

    // Health endpoints win.
    let r = client
        .get(format!("http://{addr}/healthz"))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 200);
    let body: serde_json::Value = r.json().await.unwrap();
    assert_eq!(body["status"], "ok");

    let r = client
        .get(format!("http://{addr}/readyz"))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 200);

    // Mount route wins.
    let r = client
        .get(format!("http://{addr}/status"))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 200);
    let body: serde_json::Value = r.json().await.unwrap();
    assert_eq!(body["mount"], true);

    shutdown.cancel();
    handle.await.unwrap();
}

#[tokio::test]
async fn no_fallback_yields_404() {
    let (addr, shutdown, handle) = spawn_inline_server(base_builder()).await;

    let client = reqwest::Client::new();

    let r = client.get(format!("http://{addr}/")).send().await.unwrap();
    assert_eq!(r.status(), 404);

    let r = client
        .get(format!("http://{addr}/unknown"))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 404);

    shutdown.cancel();
    handle.await.unwrap();
}

#[tokio::test]
async fn cors_applied_to_fallback() {
    let cors = tower_http::cors::CorsLayer::new()
        .allow_origin(tower_http::cors::Any)
        .allow_methods(tower_http::cors::Any)
        .allow_headers(tower_http::cors::Any);

    let (addr, shutdown, handle) =
        spawn_inline_server(base_builder().cors(cors).root_fallback(spa_router())).await;

    let resp = reqwest::Client::new()
        .get(format!("http://{addr}/"))
        .header("Origin", "http://example.com")
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    assert!(
        resp.headers().contains_key("access-control-allow-origin"),
        "fallback response should include CORS header"
    );

    shutdown.cancel();
    handle.await.unwrap();
}

#[tokio::test]
async fn auth_not_applied_to_fallback() {
    let auth_layer = BoxCloneLayer::new(axum::middleware::from_fn(
        |req: axum::http::Request<axum::body::Body>, next: axum::middleware::Next| async move {
            let authorized = req
                .headers()
                .get(axum::http::header::AUTHORIZATION)
                .and_then(|v| v.to_str().ok())
                == Some("Bearer secret");
            if authorized {
                next.run(req).await
            } else {
                axum::http::StatusCode::UNAUTHORIZED.into_response()
            }
        },
    ));

    let (addr, shutdown, handle) =
        spawn_inline_server(base_builder().auth(auth_layer).root_fallback(spa_router())).await;

    let client = reqwest::Client::new();

    // Fallback responds without auth.
    let r = client.get(format!("http://{addr}/")).send().await.unwrap();
    assert_eq!(
        r.status(),
        200,
        "fallback should be accessible without auth"
    );

    // API route requires auth.
    let r = client
        .get(format!("http://{addr}/api/v1/echo"))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 401, "API route should require auth");

    shutdown.cancel();
    handle.await.unwrap();
}

#[tokio::test]
async fn repeated_root_fallback_takes_last() {
    let first_router = Router::new().fallback(|| async { "first" });
    let second_router = Router::new().fallback(|| async { "second" });

    let (addr, shutdown, handle) = spawn_inline_server(
        base_builder()
            .root_fallback(first_router)
            .root_fallback(second_router),
    )
    .await;

    let resp = reqwest::get(format!("http://{addr}/")).await.unwrap();
    assert_eq!(resp.status(), 200);
    assert!(
        resp.text().await.unwrap().contains("second"),
        "last call should win"
    );

    shutdown.cancel();
    handle.await.unwrap();
}
