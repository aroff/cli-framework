use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::response::IntoResponse;
use axum::routing::get;
use axum::Router;
use cli_framework::api::{
    ApiServerBuilder, ApiVersion, ApiVersionName, DefaultVersion, DeprecationInfo, ReadinessReport,
    Stability,
};
use futures_util::StreamExt;
use serde_json::Value;
use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

async fn find_free_port() -> u16 {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    drop(listener);
    port
}

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

async fn spawn_server(
    builder: ApiServerBuilder,
) -> (String, tokio::task::JoinHandle<anyhow::Result<()>>) {
    let port = find_free_port().await;
    let addr = format!("127.0.0.1:{port}");
    let api = builder.build();
    let addr_for_task = addr.clone();
    let handle = tokio::spawn(async move { api.serve(&addr_for_task).await });
    wait_http_ok(&format!("http://{addr}/healthz")).await;
    (addr, handle)
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

fn version_router(tag: &'static str) -> Router {
    Router::new().route(
        "/echo",
        get(move || async move { axum::Json(serde_json::json!({"version": tag})) }),
    )
}

#[tokio::test]
async fn serves_versioned_routes_and_attaches_x_api_version() {
    let (addr, shutdown, handle) = spawn_inline_server(
        ApiServerBuilder::new()
            .version(ApiVersion {
                name: ApiVersionName::parse("v1").unwrap(),
                router: version_router("v1"),
                stability: Stability::Stable,
                deprecation: None,
            })
            .version(ApiVersion {
                name: ApiVersionName::parse("v2").unwrap(),
                router: version_router("v2"),
                stability: Stability::Stable,
                deprecation: None,
            })
            .default_version(DefaultVersion::Pinned(ApiVersionName::parse("v2").unwrap())),
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
    let body1: Value = r1.json().await.unwrap();
    assert_eq!(body1["version"], "v1");

    let r2 = client
        .get(format!("http://{addr}/api/v2/echo"))
        .send()
        .await
        .unwrap();
    assert_eq!(r2.status(), 200);
    assert_eq!(r2.headers()["X-API-Version"], "v2");
    let body2: Value = r2.json().await.unwrap();
    assert_eq!(body2["version"], "v2");

    shutdown.cancel();
    handle.await.unwrap();
}

#[tokio::test]
async fn pinned_default_redirects_unversioned_paths_and_preserves_query() {
    let (addr, shutdown, handle) = spawn_inline_server(
        ApiServerBuilder::new()
            .version(ApiVersion {
                name: ApiVersionName::parse("v1").unwrap(),
                router: version_router("v1"),
                stability: Stability::Stable,
                deprecation: None,
            })
            .version(ApiVersion {
                name: ApiVersionName::parse("v2").unwrap(),
                router: version_router("v2"),
                stability: Stability::Stable,
                deprecation: None,
            })
            .default_version(DefaultVersion::Pinned(ApiVersionName::parse("v2").unwrap())),
    )
    .await;

    let client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .unwrap();
    let r = client
        .get(format!("http://{addr}/api/echo?x=1&y=2"))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 308);
    assert_eq!(r.headers()["location"], "/api/v2/echo?x=1&y=2");

    shutdown.cancel();
    handle.await.unwrap();
}

#[tokio::test]
async fn default_none_returns_host_404_with_available_versions_and_e020() {
    let (addr, shutdown, handle) = spawn_inline_server(
        ApiServerBuilder::new()
            .version(ApiVersion {
                name: ApiVersionName::parse("v1").unwrap(),
                router: version_router("v1"),
                stability: Stability::Stable,
                deprecation: None,
            })
            .version(ApiVersion {
                name: ApiVersionName::parse("v2").unwrap(),
                router: version_router("v2"),
                stability: Stability::Stable,
                deprecation: None,
            }),
    )
    .await;

    let client = reqwest::Client::new();
    let r = client
        .get(format!("http://{addr}/api/echo"))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 404);
    let body: Value = r.json().await.unwrap();
    assert_eq!(body["error_code"], "E020");
    assert!(body["available_versions"]
        .as_array()
        .unwrap()
        .contains(&Value::from("v1")));
    assert!(body["available_versions"]
        .as_array()
        .unwrap()
        .contains(&Value::from("v2")));

    shutdown.cancel();
    handle.await.unwrap();
}

#[tokio::test]
async fn deprecated_versions_emit_deprecation_sunset_and_link_headers() {
    let sunset = chrono::Utc::now() + chrono::Duration::days(7);
    let (addr, shutdown, handle) = spawn_inline_server(
        ApiServerBuilder::new()
            .version(ApiVersion {
                name: ApiVersionName::parse("v1").unwrap(),
                router: version_router("v1"),
                stability: Stability::Stable,
                deprecation: Some(DeprecationInfo {
                    sunset,
                    docs_url: Some("https://example.com/docs".to_string()),
                }),
            })
            .default_version(DefaultVersion::Pinned(ApiVersionName::parse("v1").unwrap())),
    )
    .await;

    let client = reqwest::Client::new();
    let r = client
        .get(format!("http://{addr}/api/v1/echo"))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 200);
    assert_eq!(r.headers()["X-API-Version"], "v1");
    assert!(r.headers().get("deprecation").is_some());
    assert!(r.headers().get("sunset").is_some());
    assert!(r.headers().get("link").is_some());

    shutdown.cancel();
    handle.await.unwrap();
}

#[tokio::test]
async fn healthz_and_readyz_are_present_and_readyz_uses_readiness_check() {
    let readiness_check: cli_framework::api::ReadinessCheck = Arc::new(|| {
        Box::pin(async {
            ReadinessReport {
                ready: false,
                checks: BTreeMap::from([("db".to_string(), Value::from(false))]),
            }
        })
    });

    let (addr, shutdown, handle) = spawn_inline_server(
        ApiServerBuilder::new()
            .version(ApiVersion {
                name: ApiVersionName::parse("v1").unwrap(),
                router: version_router("v1"),
                stability: Stability::Stable,
                deprecation: None,
            })
            .readiness_check(readiness_check),
    )
    .await;

    let client = reqwest::Client::new();

    let health = client
        .get(format!("http://{addr}/healthz"))
        .send()
        .await
        .unwrap();
    assert_eq!(health.status(), 200);
    let health_json: Value = health.json().await.unwrap();
    assert_eq!(health_json["status"], "ok");
    assert_eq!(health_json["version"], env!("CARGO_PKG_VERSION"));

    let ready = client
        .get(format!("http://{addr}/readyz"))
        .send()
        .await
        .unwrap();
    assert_eq!(ready.status(), 503);
    let ready_json: Value = ready.json().await.unwrap();
    assert_eq!(ready_json["status"], "not_ready");
    assert_eq!(ready_json["error_code"], "E021");
    assert_eq!(ready_json["checks"]["db"], false);

    shutdown.cancel();
    handle.await.unwrap();
}

#[tokio::test]
async fn readyz_flips_to_503_on_sigterm_before_shutdown_completes() {
    if std::env::var("NEXTEST").as_deref() != Ok("1") {
        // This test uses SIGTERM and relies on nextest's process-per-test model.
        return;
    }
    let (addr, handle) = spawn_server(ApiServerBuilder::new().version(ApiVersion {
        name: ApiVersionName::parse("v1").unwrap(),
        router: version_router("v1"),
        stability: Stability::Stable,
        deprecation: None,
    }))
    .await;

    let client = reqwest::Client::new();
    let before = client
        .get(format!("http://{addr}/readyz"))
        .send()
        .await
        .unwrap();
    assert_eq!(before.status(), 200);

    unsafe {
        libc::kill(libc::getpid(), libc::SIGTERM);
    }

    // It should flip quickly even while graceful shutdown is in progress.
    for _ in 0..50 {
        let r = client
            .get(format!("http://{addr}/readyz"))
            .send()
            .await
            .unwrap();
        if r.status() == 503 {
            let body: Value = r.json().await.unwrap();
            assert_eq!(body["error_code"], "E021");
            handle.await.unwrap().unwrap();
            return;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("readyz did not flip to 503 after SIGTERM");
}

#[tokio::test]
async fn sse_is_streaming_and_not_buffered_by_default_middleware() {
    use axum::response::sse::{Event, Sse};

    let sse_router = Router::new().route(
        "/sse",
        get(|| async {
            let stream = futures_util::stream::unfold(0usize, |state| async move {
                match state {
                    0 => {
                        tokio::time::sleep(Duration::from_millis(150)).await;
                        Some((
                            Ok::<Event, std::convert::Infallible>(Event::default().data("one")),
                            1,
                        ))
                    }
                    1 => {
                        tokio::time::sleep(Duration::from_millis(150)).await;
                        Some((
                            Ok::<Event, std::convert::Infallible>(Event::default().data("two")),
                            2,
                        ))
                    }
                    _ => None,
                }
            });
            Sse::new(stream)
        }),
    );

    let (addr, shutdown, handle) = spawn_inline_server(
        ApiServerBuilder::new()
            .version(ApiVersion {
                name: ApiVersionName::parse("v1").unwrap(),
                router: sse_router,
                stability: Stability::Stable,
                deprecation: None,
            })
            .default_version(DefaultVersion::Pinned(ApiVersionName::parse("v1").unwrap())),
    )
    .await;

    let client = reqwest::Client::new();
    let resp = client
        .get(format!("http://{addr}/api/v1/sse"))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    assert_eq!(resp.headers()["X-API-Version"], "v1");

    let mut stream = resp.bytes_stream();
    let first = tokio::time::timeout(Duration::from_secs(1), stream.next())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    let first_str = String::from_utf8_lossy(&first);
    assert!(first_str.contains("one"));

    shutdown.cancel();
    handle.await.unwrap();
}

#[tokio::test]
async fn websocket_upgrade_succeeds_and_includes_x_api_version() {
    async fn ws_handler(ws: WebSocketUpgrade) -> impl IntoResponse {
        ws.on_upgrade(|mut socket: WebSocket| async move {
            let _ = socket.send(Message::Text("hello".into())).await;
        })
    }

    let ws_router = Router::new().route("/ws", get(ws_handler));

    let (addr, shutdown, handle) = spawn_inline_server(
        ApiServerBuilder::new()
            .version(ApiVersion {
                name: ApiVersionName::parse("v1").unwrap(),
                router: ws_router,
                stability: Stability::Stable,
                deprecation: None,
            })
            .default_version(DefaultVersion::Pinned(ApiVersionName::parse("v1").unwrap())),
    )
    .await;

    // First: verify handshake has X-API-Version.
    let client = reqwest::Client::new();
    let handshake = client
        .get(format!("http://{addr}/api/v1/ws"))
        .header("connection", "upgrade")
        .header("upgrade", "websocket")
        .header("sec-websocket-version", "13")
        .header("sec-websocket-key", "dGhlIHNhbXBsZSBub25jZQ==")
        .send()
        .await
        .unwrap();
    assert_eq!(handshake.status(), 101);
    assert_eq!(handshake.headers()["X-API-Version"], "v1");

    // Then: make a real websocket connection.
    let url = reqwest::Url::parse(&format!("ws://{addr}/api/v1/ws")).unwrap();
    let (mut ws, _resp) = tokio_tungstenite::connect_async(url).await.unwrap();
    let msg = ws.next().await.unwrap().unwrap();
    assert_eq!(msg.into_text().unwrap(), "hello");

    shutdown.cancel();
    handle.await.unwrap();
}

#[test]
fn build_validation_panics_with_stable_error_codes() {
    fn panic_message<T: std::fmt::Debug>(p: std::thread::Result<T>) -> String {
        match p {
            Ok(v) => format!("expected panic, got: {v:?}"),
            Err(e) => {
                if let Some(s) = e.downcast_ref::<&str>() {
                    s.to_string()
                } else if let Some(s) = e.downcast_ref::<String>() {
                    s.clone()
                } else {
                    format!("{e:?}")
                }
            }
        }
    }

    let ok_router = Router::new();

    // E017: zero versions
    let p = std::panic::catch_unwind(|| ApiServerBuilder::new().build());
    assert!(panic_message(p).contains("E017"));

    // E016: pinned unknown version
    let p = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        ApiServerBuilder::new()
            .version(ApiVersion {
                name: ApiVersionName::parse("v1").unwrap(),
                router: ok_router.clone(),
                stability: Stability::Stable,
                deprecation: None,
            })
            .default_version(DefaultVersion::Pinned(ApiVersionName::parse("v2").unwrap()))
            .build()
    }));
    assert!(panic_message(p).contains("E016"));

    // E015: duplicate versions
    let p = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        ApiServerBuilder::new()
            .version(ApiVersion {
                name: ApiVersionName::parse("v1").unwrap(),
                router: ok_router.clone(),
                stability: Stability::Stable,
                deprecation: None,
            })
            .version(ApiVersion {
                name: ApiVersionName::parse("v1").unwrap(),
                router: ok_router.clone(),
                stability: Stability::Stable,
                deprecation: None,
            })
            .build()
    }));
    assert!(panic_message(p).contains("E015"));

    // E014: invalid version name
    let p = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        ApiServerBuilder::new()
            .version(ApiVersion {
                name: ApiVersionName::new_unchecked("nope"),
                router: ok_router.clone(),
                stability: Stability::Stable,
                deprecation: None,
            })
            .build()
    }));
    assert!(panic_message(p).contains("E014"));

    // E019: reserved api segment
    let p = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        ApiServerBuilder::new()
            .version(ApiVersion {
                name: ApiVersionName::new_unchecked("docs"),
                router: ok_router.clone(),
                stability: Stability::Stable,
                deprecation: None,
            })
            .build()
    }));
    assert!(panic_message(p).contains("E019"));

    // E018: mount collision
    let p = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        ApiServerBuilder::new()
            .version(ApiVersion {
                name: ApiVersionName::parse("v1").unwrap(),
                router: ok_router.clone(),
                stability: Stability::Stable,
                deprecation: None,
            })
            .mount("/healthz", Router::new())
            .build()
    }));
    assert!(panic_message(p).contains("E018"));

    // E018: /mcp is reserved (use ApiServerBuilder::mcp_router instead)
    let p = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        ApiServerBuilder::new()
            .version(ApiVersion {
                name: ApiVersionName::parse("v1").unwrap(),
                router: ok_router.clone(),
                stability: Stability::Stable,
                deprecation: None,
            })
            .mount("/mcp", Router::new())
            .build()
    }));
    assert!(panic_message(p).contains("E018"));
}
