use axum::routing::get;
use cli_framework::command::{Command, CommandRegistry};
use cli_framework::mcp::{
    build_mcp_axum_router, transport_http::mcp_axum_router, McpToolExportPolicy, McpToolRegistry,
};
use cli_framework::security::CommandRiskPolicy;
use cli_framework::spec::command_tree::CommandSpec;
use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

fn noop_execute() -> Arc<
    dyn for<'a> Fn(
            &'a mut dyn cli_framework::app::AppContext,
            HashMap<String, cli_framework::spec::value::ArgValue>,
        ) -> Pin<Box<dyn Future<Output = anyhow::Result<()>> + Send + 'a>>
        + Send
        + Sync,
> {
    Arc::new(|_ctx, _args| Box::pin(async { Ok(()) }))
}

fn make_command(id: &'static str, summary: &'static str, expose_mcp: bool) -> Command {
    Command {
        id: Arc::from(id),
        spec: Arc::new(CommandSpec {
            summary,
            ..Default::default()
        }),
        validator: None,
        expose_mcp,
        expose_chat: true,
        execute: noop_execute(),
    }
}

async fn wait_for_server(addr: &str) {
    for _ in 0..50 {
        if tokio::net::TcpStream::connect(addr).await.is_ok() {
            return;
        }
        tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
    }
    panic!("Server did not start within 5s at {}", addr);
}

fn parse_sse_data(body: &str) -> serde_json::Value {
    for line in body.lines() {
        if let Some(data) = line.strip_prefix("data: ") {
            if let Ok(v) = serde_json::from_str(data) {
                return v;
            }
        }
    }
    serde_json::from_str(body).unwrap_or(serde_json::Value::Null)
}

async fn initialize_session(client: &reqwest::Client, base_url: &str) -> Option<String> {
    let resp = client
        .post(format!("{}/mcp", base_url))
        .header("Content-Type", "application/json")
        .header("Accept", "application/json, text/event-stream")
        .json(&serde_json::json!({
            "jsonrpc": "2.0",
            "id": "1",
            "method": "initialize",
            "params": {
                "protocolVersion": "2024-11-05",
                "capabilities": {},
                "clientInfo": {"name": "test-client", "version": "0.1.0"}
            }
        }))
        .send()
        .await
        .expect("initialize request failed");

    let session_id = resp
        .headers()
        .get("Mcp-Session-Id")
        .or_else(|| resp.headers().get("mcp-session-id"))
        .map(|v| v.to_str().unwrap_or("").to_string());

    let _body = resp.text().await.unwrap_or_default();

    session_id
}

#[tokio::test]
async fn test_mcp_and_health_on_same_listener() {
    let _ = env_logger::try_init();

    let mut registry = CommandRegistry::new();
    registry.register(make_command("hello", "Say hello", false));

    let tool_registry = Arc::new(McpToolRegistry::from_command_registry(&registry, "testapp"));
    let mcp_router = mcp_axum_router(tool_registry, "/mcp");

    let app = axum::Router::new()
        .route("/health", get(|| async { "OK" }))
        .merge(mcp_router);

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();

    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });

    wait_for_server(&format!("127.0.0.1:{}", port)).await;

    let client = reqwest::Client::new();
    let base_url = format!("http://127.0.0.1:{}", port);

    // Verify health route
    let health_resp = client
        .get(format!("{}/health", base_url))
        .send()
        .await
        .expect("health request failed");
    assert!(
        health_resp.status().is_success(),
        "health status: {}",
        health_resp.status()
    );

    // Verify MCP tools/list
    let session_id = initialize_session(&client, &base_url).await;

    let mut req = client
        .post(format!("{}/mcp", base_url))
        .header("Content-Type", "application/json")
        .header("Accept", "application/json, text/event-stream");

    if let Some(ref sid) = session_id {
        req = req.header("Mcp-Session-Id", sid);
    }

    let resp = req
        .json(&serde_json::json!({
            "jsonrpc": "2.0",
            "id": "2",
            "method": "tools/list"
        }))
        .send()
        .await
        .expect("tools/list request failed");

    assert!(
        resp.status().is_success(),
        "tools/list status: {}",
        resp.status()
    );

    let body = resp.text().await.unwrap();
    let json = parse_sse_data(&body);

    let tools = json
        .pointer("/result/tools")
        .and_then(|t| t.as_array())
        .expect("result.tools array expected");

    assert_eq!(tools.len(), 1, "expected 1 tool, got {}", tools.len());
}

#[tokio::test]
async fn test_build_mcp_axum_router_tool_count() {
    let _ = env_logger::try_init();

    let mut registry = CommandRegistry::new();
    registry.register(make_command("foo", "Foo command", false));
    registry.register(make_command("bar", "Bar command", false));

    let mcp_router = build_mcp_axum_router(
        &registry,
        "testapp",
        "/mcp",
        CommandRiskPolicy::default(),
        McpToolExportPolicy::AllCommands,
    );
    let app = axum::Router::new().merge(mcp_router);

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();

    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });

    wait_for_server(&format!("127.0.0.1:{}", port)).await;

    let client = reqwest::Client::new();
    let base_url = format!("http://127.0.0.1:{}", port);

    let session_id = initialize_session(&client, &base_url).await;

    let mut req = client
        .post(format!("{}/mcp", base_url))
        .header("Content-Type", "application/json")
        .header("Accept", "application/json, text/event-stream");

    if let Some(ref sid) = session_id {
        req = req.header("Mcp-Session-Id", sid);
    }

    let resp = req
        .json(&serde_json::json!({
            "jsonrpc": "2.0",
            "id": "2",
            "method": "tools/list"
        }))
        .send()
        .await
        .expect("tools/list request failed");

    assert!(
        resp.status().is_success(),
        "tools/list status: {}",
        resp.status()
    );

    let body = resp.text().await.unwrap();
    let json = parse_sse_data(&body);

    let tools = json
        .pointer("/result/tools")
        .and_then(|t| t.as_array())
        .expect("result.tools array expected");

    assert_eq!(tools.len(), 2, "expected 2 tools, got {}", tools.len());
}

#[tokio::test]
async fn test_expose_mcp_only_via_http_router() {
    let _ = env_logger::try_init();

    let mut registry = CommandRegistry::new();
    registry.register(make_command("visible", "Visible via MCP", true));
    registry.register(make_command("hidden", "Hidden from MCP", false));

    let mcp_router = build_mcp_axum_router(
        &registry,
        "testapp",
        "/mcp",
        CommandRiskPolicy::default(),
        McpToolExportPolicy::ExposeMcpOnly,
    );
    let app = axum::Router::new().merge(mcp_router);

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();

    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });

    wait_for_server(&format!("127.0.0.1:{}", port)).await;

    let client = reqwest::Client::new();
    let base_url = format!("http://127.0.0.1:{}", port);

    let session_id = initialize_session(&client, &base_url).await;

    let mut req = client
        .post(format!("{}/mcp", base_url))
        .header("Content-Type", "application/json")
        .header("Accept", "application/json, text/event-stream");

    if let Some(ref sid) = session_id {
        req = req.header("Mcp-Session-Id", sid);
    }

    let resp = req
        .json(&serde_json::json!({
            "jsonrpc": "2.0",
            "id": "2",
            "method": "tools/list"
        }))
        .send()
        .await
        .expect("tools/list request failed");

    assert!(
        resp.status().is_success(),
        "tools/list status: {}",
        resp.status()
    );

    let body = resp.text().await.unwrap();
    let json = parse_sse_data(&body);

    let tools = json
        .pointer("/result/tools")
        .and_then(|t| t.as_array())
        .expect("result.tools array expected");

    assert_eq!(
        tools.len(),
        1,
        "expected 1 tool under ExposeMcpOnly, got {}",
        tools.len()
    );

    let tool_name = tools[0]
        .get("name")
        .and_then(|n| n.as_str())
        .expect("tool name expected");
    assert!(
        tool_name.contains("visible"),
        "expected visible command in tool list, got: {}",
        tool_name
    );
}
