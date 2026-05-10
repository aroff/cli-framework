use cli_framework::app::{AppBuilder, AppContext};
use cli_framework::command::{Command, CommandArgs, CommandRegistry};
use cli_framework::mcp::{
    serve_mcp, CliFrameworkHandler, McpServerArgs, McpToolExportPolicy, McpToolRegistry,
    McpTransportKind,
};
use cli_framework::security::CommandRiskPolicy;
use cli_framework::spec::command_tree::CommandSpec;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

fn noop_execute() -> Arc<
    dyn for<'a> Fn(
            &'a mut dyn cli_framework::app::AppContext,
            CommandArgs,
        ) -> Pin<Box<dyn Future<Output = anyhow::Result<()>> + Send + 'a>>
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
        expose_mcp: false,
        execute: noop_execute(),
    });
    registry.register(Command {
        id: "goodbye",
        summary: "Say goodbye",
        syntax: None,
        category: None,
        spec: None,
        validator: None,
        expose_mcp: false,
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
            McpToolExportPolicy::AllCommands,
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
        expose_mcp: false,
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
            McpToolExportPolicy::AllCommands,
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

/// Stage 2 requirement: `prog mcp serve --port <ephemeral>` starts the server via the subcommand
/// dispatch path and exposes registered commands as MCP tools.
#[tokio::test]
async fn test_mcp_serve_subcommand_tools_list() {
    let _ = env_logger::try_init();

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    drop(listener);

    // Construct and start the app via `mcp serve` subcommand in a background thread
    // so the blocking serve call does not stall the test.
    std::thread::spawn(move || {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("tokio runtime");
        rt.block_on(async move {
            struct Ctx;
            impl AppContext for Ctx {}

            let mut app = AppBuilder::new()
                .with_version("testapp", "0.1.0")
                .register_command(Command {
                    id: "widget",
                    summary: "Widget command exposed via mcp serve subcommand",
                    syntax: None,
                    category: None,
                    spec: Some(Arc::new(CommandSpec {
                        summary: "Widget command exposed via mcp serve subcommand",
                        ..Default::default()
                    })),
                    validator: None,
                    expose_mcp: true,
                    execute: Arc::new(|_ctx, _args| Box::pin(async { Ok(()) })),
                })
                .unwrap()
                .build(Ctx)
                .unwrap();

            let _ = app
                .run_with_args(vec![
                    "testapp".to_string(),
                    "mcp".to_string(),
                    "serve".to_string(),
                    "--port".to_string(),
                    port.to_string(),
                ])
                .await;
        });
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

    assert!(
        tools
            .iter()
            .any(|t| t["name"].as_str() == Some("testapp.widget")),
        "testapp.widget not found in tools: {:?}",
        tools
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
    let result = serve_mcp(
        registry,
        "testapp",
        args,
        CommandRiskPolicy::default(),
        McpToolExportPolicy::AllCommands,
    )
    .await;

    assert!(result.is_err(), "expected bind failure error");
    let err_msg = result.unwrap_err().to_string();
    assert!(
        err_msg.contains("MCP_BIND_FAILED"),
        "error should contain MCP_BIND_FAILED, got: {}",
        err_msg
    );

    drop(listener);
}

#[tokio::test]
async fn test_tools_list_and_call_over_stdio_transport() {
    let _ = env_logger::try_init();

    let mut registry = CommandRegistry::new();
    registry.register(Command {
        id: "hello",
        summary: "Say hello",
        syntax: None,
        category: None,
        spec: None,
        validator: None,
        expose_mcp: false,
        execute: noop_execute(),
    });
    let registry = Arc::new(registry);

    let tool_registry = Arc::new(
        McpToolRegistry::from_command_registry_with_policy(
            &registry,
            "testapp",
            McpToolExportPolicy::AllCommands,
        )
        .with_risk_policy(CommandRiskPolicy::default()),
    );

    // Use an in-memory duplex stream to simulate stdio (no TCP).
    let (server_stream, client_stream) = tokio::io::duplex(64 * 1024);

    let serialize = std::sync::Arc::new(tokio::sync::Mutex::new(()));
    let server_task = tokio::spawn(async move {
        rmcp::serve_server(
            CliFrameworkHandler::new(tool_registry, McpTransportKind::Stdio)
                .with_stdio_serialization(serialize),
            server_stream,
        )
        .await
    });

    let client = rmcp::serve_client((), client_stream)
        .await
        .expect("serve_client failed");

    let server = server_task
        .await
        .expect("server task join")
        .expect("serve_server (stdio-like) failed");

    let tools = client
        .peer()
        .list_tools(Default::default())
        .await
        .expect("tools/list failed");

    assert!(
        tools.tools.iter().any(|t| t.name == "testapp.hello"),
        "expected testapp.hello in tools: {:?}",
        tools.tools
    );

    let call = client
        .peer()
        .call_tool(rmcp::model::CallToolRequestParams::new("testapp.hello"))
        .await
        .expect("tools/call failed");

    assert_eq!(call.is_error, Some(false));

    let _ = client.cancel().await;
    let _ = server.cancel().await;
}

#[tokio::test]
async fn test_mcp_serve_stdio_rejects_http_flags() {
    struct Ctx;
    impl AppContext for Ctx {}

    let mut app = AppBuilder::new()
        .with_version("testapp", "0.1.0")
        .build(Ctx)
        .unwrap();

    let result = app
        .run_with_args(vec![
            "testapp".to_string(),
            "mcp".to_string(),
            "serve".to_string(),
            "--transport".to_string(),
            "stdio".to_string(),
            "--port".to_string(),
            "9999".to_string(),
        ])
        .await;

    assert!(result.is_err(), "expected error for invalid stdio usage");
    let msg = result.unwrap_err().to_string();
    assert!(msg.contains("E004"), "expected E004, got: {}", msg);
}
