use axum::routing::get;
use axum::Router;
use cli_framework::api::{ApiServerBuilder, ApiVersion, ApiVersionName, Stability};
use cli_framework::command::CommandRegistry;
use cli_framework::mcp::transport_http::mcp_axum_router;
use cli_framework::mcp::McpToolRegistry;
use std::sync::Arc;
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

#[tokio::test]
async fn mcp_can_be_mounted_under_fixed_mcp_prefix() {
    let v1 = Router::new().route("/echo", get(|| async { "ok" }));

    let registry = CommandRegistry::new();
    let tool_registry = Arc::new(McpToolRegistry::from_command_registry(&registry, "test"));
    let mcp_fragment = mcp_axum_router(tool_registry, "/mcp");

    let api = ApiServerBuilder::new()
        .version(ApiVersion {
            name: ApiVersionName::parse("v1").unwrap(),
            router: v1,
            stability: Stability::Stable,
            deprecation: None,
        })
        .mcp_router(mcp_fragment)
        .build();

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

    let client = reqwest::Client::new();
    let r = client
        .get(format!("http://{addr_str}/mcp"))
        .send()
        .await
        .unwrap();
    assert_ne!(r.status(), reqwest::StatusCode::NOT_FOUND);

    shutdown.cancel();
    handle.await.unwrap();
}
