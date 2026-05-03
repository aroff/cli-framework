use cli_framework::command::{Command, CommandArgs, CommandRegistry};
use cli_framework::mcp::{serve_mcp, McpServerArgs};
use cli_framework::security::CommandRiskPolicy;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

fn noop_execute() -> Arc<
    dyn Fn(
            &mut dyn cli_framework::app::AppContext,
            CommandArgs,
        ) -> Pin<Box<dyn Future<Output = anyhow::Result<()>> + Send>>
        + Send
        + Sync,
> {
    Arc::new(|_ctx, _args| Box::pin(async { Ok(()) }))
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
    // Try parsing as plain JSON
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

    // Consume the response body
    let _body = resp.text().await.unwrap_or_default();

    session_id
}

#[tokio::test]
async fn test_tools_list_over_http() {
    let _ = env_logger::try_init();

    let mut registry = CommandRegistry::new();
    registry.register(Command {
        id: "hello",
        summary: "Say hello",
        syntax: None,
        category: None,
        spec: None,
        validator: None,
        execute: noop_execute(),
    });
    registry.register(Command {
        id: "goodbye",
        summary: "Say goodbye",
        syntax: None,
        category: None,
        spec: None,
        validator: None,
        execute: noop_execute(),
    });

    let registry = Arc::new(registry);

    // Find ephemeral port
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    drop(listener);

    let args = McpServerArgs {
        host: "127.0.0.1".to_string(),
        port,
        path: "/mcp".to_string(),
    };

    let registry_clone = Arc::clone(&registry);
    let args_clone = args.clone();
    tokio::spawn(async move {
        let _ = serve_mcp(
            registry_clone,
            "testapp",
            args_clone,
            CommandRiskPolicy::default(),
        )
        .await;
    });

    wait_for_server(&format!("127.0.0.1:{}", port)).await;

    let client = reqwest::Client::new();
    let base_url = format!("http://127.0.0.1:{}", port);

    let session_id = initialize_session(&client, &base_url).await;

    // Send tools/list
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

    // Verify we got a tools list result
    let tools = json
        .pointer("/result/tools")
        .and_then(|t| t.as_array())
        .expect("result.tools array expected");

    assert_eq!(tools.len(), 2, "expected 2 tools, got {}", tools.len());

    let names: Vec<&str> = tools.iter().filter_map(|t| t["name"].as_str()).collect();
    assert!(
        names.contains(&"testapp.hello"),
        "testapp.hello not found in {:?}",
        names
    );
    assert!(
        names.contains(&"testapp.goodbye"),
        "testapp.goodbye not found in {:?}",
        names
    );
}

#[tokio::test]
async fn test_tool_call_success_over_http() {
    let _ = env_logger::try_init();

    let mut registry = CommandRegistry::new();
    registry.register(Command {
        id: "ping",
        summary: "Ping command",
        syntax: None,
        category: None,
        spec: None,
        validator: None,
        execute: noop_execute(),
    });

    let registry = Arc::new(registry);

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    drop(listener);

    let args = McpServerArgs {
        host: "127.0.0.1".to_string(),
        port,
        path: "/mcp".to_string(),
    };

    let registry_clone = Arc::clone(&registry);
    let args_clone = args.clone();
    tokio::spawn(async move {
        let _ = serve_mcp(
            registry_clone,
            "testapp",
            args_clone,
            CommandRiskPolicy::default(),
        )
        .await;
    });

    wait_for_server(&format!("127.0.0.1:{}", port)).await;

    let client = reqwest::Client::new();
    let base_url = format!("http://127.0.0.1:{}", port);

    let session_id = initialize_session(&client, &base_url).await;

    // Call the tool
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
            "id": "3",
            "method": "tools/call",
            "params": {
                "name": "testapp.ping",
                "arguments": {}
            }
        }))
        .send()
        .await
        .expect("tools/call request failed");

    assert!(
        resp.status().is_success(),
        "tools/call status: {}",
        resp.status()
    );

    let body = resp.text().await.unwrap();
    let json = parse_sse_data(&body);

    // Verify successful tool call
    assert!(
        json.pointer("/result").is_some() || json.pointer("/error").is_none(),
        "unexpected error in response: {}",
        json
    );
}

#[tokio::test]
async fn test_bind_failure() {
    let _ = env_logger::try_init();

    // Bind a port to occupy it
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();

    let registry = Arc::new(CommandRegistry::new());
    let args = McpServerArgs {
        host: "127.0.0.1".to_string(),
        port,
        path: "/mcp".to_string(),
    };

    // Try to start the MCP server on the already-bound port
    let result = serve_mcp(registry, "testapp", args, CommandRiskPolicy::default()).await;

    assert!(result.is_err(), "expected bind failure error");
    let err_msg = result.unwrap_err().to_string();
    assert!(
        err_msg.contains("MCP_BIND_FAILED"),
        "error should contain MCP_BIND_FAILED, got: {}",
        err_msg
    );

    drop(listener);
}
