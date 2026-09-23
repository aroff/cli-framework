use cli_framework::app::{AppBuilder, AppContext, RequestIdentityExt};
use cli_framework::command::{Command, CommandRegistry};
use cli_framework::mcp::{
    serve_mcp_with_gate, CliFrameworkHandler, McpDynamicTool, McpDynamicToolProvider,
    McpServerArgs, McpToolExportPolicy, McpToolPresentation, McpToolRegistry, McpTransportKind,
};
use cli_framework::security::CommandRiskPolicy;
use cli_framework::spec::command_tree::CommandSpec;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

fn noop_execute() -> Arc<
    dyn for<'a> Fn(
            &'a mut dyn cli_framework::app::AppContext,
            std::collections::HashMap<String, cli_framework::spec::value::ArgValue>,
        ) -> Pin<Box<dyn Future<Output = anyhow::Result<()>> + Send + 'a>>
        + Send
        + Sync,
> {
    Arc::new(|_ctx, _args| Box::pin(async { Ok(()) }))
}

/// Serialises "reserve a port, then start a server on it".
///
/// The servers under test are started from an argument list
/// (`mcp serve --port N`) and cannot be handed an already-bound socket, so a
/// port must be chosen, released, and then re-bound by somebody else. That
/// window is unavoidable. All this lock does is keep two tests from being
/// inside it on the same port number at the same time; [`reserve_port`] handles
/// the harder half -- making sure nothing *else* can be handed the port while
/// the window is open.
///
/// This is a `tokio` mutex rather than a `std` one because every holder awaits
/// while holding it, which `clippy::await_holding_lock` rightly rejects for a
/// `std::sync::MutexGuard`.
fn port_lock() -> &'static tokio::sync::Mutex<()> {
    static LOCK: std::sync::OnceLock<tokio::sync::Mutex<()>> = std::sync::OnceLock::new();
    LOCK.get_or_init(|| tokio::sync::Mutex::new(()))
}

/// Reserves a free TCP port from a range the kernel never assigns on its own.
///
/// The obvious way to pick a port -- `bind("127.0.0.1:0")`, read
/// `local_addr()`, drop the listener -- draws from `ip_local_port_range`
/// (`32768 60999` on this box), which is the same pool the kernel draws
/// outbound source ports from. libtest runs every test in a binary on threads
/// of one process, so while one test sits in the release-then-rebind window
/// another test's `reqwest` connection can be assigned that exact port, and the
/// server's bind then fails with `EADDRINUSE`. Nor is it a
/// one-in-thirty-thousand coincidence: the kernel scans that pool from a
/// rotating offset, so the port just released is frequently the very next one
/// handed out.
///
/// This was observed, not theorised. Serialising the *allocation* with
/// [`port_lock`] was tried first and was not enough -- the suite still failed
/// with
///
/// ```text
/// Server did not become HTTP-ready within 10s at http://127.0.0.1:40629.
/// Background server exits: mcp serve: MCP_BIND_FAILED: address
/// 127.0.0.1:40629 already in use: Address already in use (os error 98)
/// ```
///
/// because a lock over this file's allocations has no authority over another
/// test's outbound connections. Choosing from 20000-30000 removes the mechanism
/// rather than narrowing its window: that range sits below Linux's default
/// ephemeral floor (32768) and below macOS's (49152), so nothing is given one
/// of these ports unless it asks for that number. Only this helper asks, and
/// [`port_lock`] serialises it.
///
/// The probe bind proves the candidate is free right now; a listening socket
/// that never accepted anything leaves no `TIME_WAIT` behind, so the server can
/// take it immediately. Ports stay held for the life of the process -- the
/// servers these tests start are detached and never shut down -- so the scan
/// walks past them, and the per-call counter plus the pid offset keep
/// concurrently running test binaries from starting their scans in the same
/// place.
fn reserve_port() -> u16 {
    use std::sync::atomic::{AtomicU16, Ordering};

    const BASE: u32 = 20_000;
    const SPAN: u32 = 10_000;
    static NEXT: AtomicU16 = AtomicU16::new(0);

    let seed = std::process::id() as u32 + NEXT.fetch_add(1, Ordering::Relaxed) as u32;
    for offset in 0..SPAN {
        let candidate = (BASE + (seed + offset) % SPAN) as u16;
        if let Ok(listener) = std::net::TcpListener::bind(("127.0.0.1", candidate)) {
            drop(listener);
            return candidate;
        }
    }
    panic!("no free TCP port in {}..{}", BASE, BASE + SPAN);
}

/// Why a background server exited, when one did.
///
/// The servers in this file run detached -- on a `tokio` task or on an OS
/// thread -- so their `Result` has nowhere to return to and used to be dropped
/// with `let _ =`. That is what turned "the port was taken, so bind failed"
/// into a bare ten-second readiness timeout with no cause attached. Recording
/// the error here lets [`wait_for_http_server`] report it, so a future
/// occurrence is diagnosed from its own output instead of by inference. It
/// earned its keep immediately: it is what showed that serialising allocation
/// alone did not fix the flake, which is the finding [`reserve_port`] rests on.
static SERVER_EXITS: std::sync::Mutex<Vec<String>> = std::sync::Mutex::new(Vec::new());

fn record_server_exit<T, E: std::fmt::Display>(label: &str, result: Result<T, E>) {
    if let Err(err) = result {
        SERVER_EXITS
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .push(format!("{}: {}", label, err));
    }
}

fn server_exits() -> String {
    let exits = SERVER_EXITS
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if exits.is_empty() {
        "none recorded (no background server returned an error)".to_string()
    } else {
        exits.join("; ")
    }
}

/// Wait until the server is ready at the HTTP level, not just TCP level.
/// Returns the session ID from the initialize handshake.
///
/// Checking TCP connectivity only (connect + drop) creates a TOCTOU race: the OS
/// kernel accepts the SYN before axum has finished wiring its router, so a
/// subsequent HTTP request can still get "Connection refused".  Probing with an
/// actual HTTP initialize avoids that window entirely.
///
/// The probe request carries its own 500 ms timeout, and the loop is bounded by
/// wall-clock rather than by an attempt count. Both matter: a port that is bound
/// but never answers -- the kernel completes the TCP handshake out of the listen
/// backlog whether or not anything ever calls `accept` -- leaves an untimed
/// `send()` awaiting a response that never arrives, so the ten-second bound
/// never gets a chance to fire and the test hangs until the harness kills it.
/// An attempt count is not a time bound when a single attempt can block forever.
async fn wait_for_http_server(client: &reqwest::Client, base_url: &str) -> Option<String> {
    let deadline = tokio::time::Instant::now() + tokio::time::Duration::from_secs(10);
    while tokio::time::Instant::now() < deadline {
        match client
            .post(format!("{}/mcp", base_url))
            .header("Content-Type", "application/json")
            .header("Accept", "application/json, text/event-stream")
            .json(&serde_json::json!({
                "jsonrpc": "2.0",
                "id": "probe",
                "method": "initialize",
                "params": {
                    "protocolVersion": "2024-11-05",
                    "capabilities": {},
                    "clientInfo": {"name": "probe", "version": "0"}
                }
            }))
            .timeout(std::time::Duration::from_millis(500))
            .send()
            .await
        {
            Ok(resp) if resp.status().is_success() => {
                let session_id = resp
                    .headers()
                    .get("Mcp-Session-Id")
                    .or_else(|| resp.headers().get("mcp-session-id"))
                    .map(|v| v.to_str().unwrap_or("").to_string());
                let _ = resp.text().await;
                return session_id;
            }
            _ => {}
        }
        tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
    }
    panic!(
        "Server did not become HTTP-ready within 10s at {}. \
         Background server exits: {}",
        base_url,
        server_exits()
    );
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

#[allow(dead_code)]
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
        id: Arc::from("hello"),
        spec: Arc::new(CommandSpec {
            summary: "Say hello",
            ..Default::default()
        }),
        validator: None,
        expose_mcp: false,
        expose_chat: true,
        meta: None,
        visibility: None,
        execute: noop_execute(),
    });
    registry.register(Command {
        id: Arc::from("goodbye"),
        spec: Arc::new(CommandSpec {
            summary: "Say goodbye",
            ..Default::default()
        }),
        validator: None,
        expose_mcp: false,
        expose_chat: true,
        meta: None,
        visibility: None,
        execute: noop_execute(),
    });

    let registry = Arc::new(registry);

    // The guard is held until the server owns the port; see `port_lock`.
    let port_guard = port_lock().lock().await;
    let port = reserve_port();

    let args = McpServerArgs {
        host: "127.0.0.1".to_string(),
        port,
        path: "/mcp".to_string(),
    };

    let registry_clone = Arc::clone(&registry);
    let args_clone = args.clone();
    tokio::spawn(async move {
        record_server_exit(
            "serve_mcp_with_gate",
            serve_mcp_with_gate(
                registry_clone,
                "testapp",
                args_clone,
                CommandRiskPolicy::default(),
                McpToolExportPolicy::AllCommands,
                None,
            )
            .await,
        );
    });

    let client = reqwest::Client::new();
    let base_url = format!("http://127.0.0.1:{}", port);
    let session_id = wait_for_http_server(&client, &base_url).await;
    // The server owns the port now; the next test may allocate.
    drop(port_guard);

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
        names.contains(&"testapp_hello"),
        "testapp_hello not found in {:?}",
        names
    );
    assert!(
        names.contains(&"testapp_goodbye"),
        "testapp_goodbye not found in {:?}",
        names
    );
}

#[tokio::test]
async fn test_tool_call_success_over_http() {
    let _ = env_logger::try_init();

    let mut registry = CommandRegistry::new();
    registry.register(Command {
        id: Arc::from("ping"),
        spec: Arc::new(CommandSpec {
            summary: "Ping command",
            ..Default::default()
        }),
        validator: None,
        expose_mcp: false,
        expose_chat: true,
        meta: None,
        visibility: None,
        execute: noop_execute(),
    });

    let registry = Arc::new(registry);

    let port_guard = port_lock().lock().await;
    let port = reserve_port();

    let args = McpServerArgs {
        host: "127.0.0.1".to_string(),
        port,
        path: "/mcp".to_string(),
    };

    let registry_clone = Arc::clone(&registry);
    let args_clone = args.clone();
    tokio::spawn(async move {
        record_server_exit(
            "serve_mcp_with_gate",
            serve_mcp_with_gate(
                registry_clone,
                "testapp",
                args_clone,
                CommandRiskPolicy::default(),
                McpToolExportPolicy::AllCommands,
                None,
            )
            .await,
        );
    });

    let client = reqwest::Client::new();
    let base_url = format!("http://127.0.0.1:{}", port);
    let session_id = wait_for_http_server(&client, &base_url).await;
    // The server owns the port now; the next test may allocate.
    drop(port_guard);

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
                "name": "testapp_ping",
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

    let port_guard = port_lock().lock().await;
    let port = reserve_port();

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
                    id: Arc::from("widget"),
                    spec: Arc::new(CommandSpec {
                        summary: "Widget command exposed via mcp serve subcommand",
                        ..Default::default()
                    }),
                    validator: None,
                    expose_mcp: true,
                    expose_chat: true,
                    meta: None,
                    visibility: None,
                    execute: Arc::new(|_ctx, _args| Box::pin(async { Ok(()) })),
                })
                .unwrap()
                .build(Ctx)
                .unwrap();

            record_server_exit(
                "mcp serve",
                app.run_with_args(vec![
                    "testapp".to_string(),
                    "mcp".to_string(),
                    "serve".to_string(),
                    "--port".to_string(),
                    port.to_string(),
                ])
                .await,
            );
        });
    });

    let client = reqwest::Client::new();
    let base_url = format!("http://127.0.0.1:{}", port);
    // wait_for_http_server combines the TCP wait + initialize in one HTTP retry loop,
    // eliminating the TOCTOU race between TCP-ready and HTTP-handler-ready.
    let session_id = wait_for_http_server(&client, &base_url).await;
    // The server owns the port now; the next test may allocate.
    drop(port_guard);

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
            .any(|t| t["name"].as_str() == Some("testapp_widget")),
        "testapp_widget not found in tools: {:?}",
        tools
    );
}

#[tokio::test]
async fn test_bind_failure() {
    let _ = env_logger::try_init();

    // Bind a port to occupy it. This test keeps its listener for the whole test,
    // so it never enters the release-then-rebind window `reserve_port` exists to
    // avoid, and takes no part in that protocol: the port it holds is precisely
    // the port the server under test must fail to bind.
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();

    let registry = Arc::new(CommandRegistry::new());
    let args = McpServerArgs {
        host: "127.0.0.1".to_string(),
        port,
        path: "/mcp".to_string(),
    };

    // Try to start the MCP server on the already-bound port
    let result = serve_mcp_with_gate(
        registry,
        "testapp",
        args,
        CommandRiskPolicy::default(),
        McpToolExportPolicy::AllCommands,
        None,
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
        id: Arc::from("hello"),
        spec: Arc::new(CommandSpec {
            summary: "Say hello",
            ..Default::default()
        }),
        validator: None,
        expose_mcp: false,
        expose_chat: true,
        meta: None,
        visibility: None,
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
        tools.tools.iter().any(|t| t.name == "testapp_hello"),
        "expected testapp_hello in tools: {:?}",
        tools.tools
    );

    let call = client
        .peer()
        .call_tool(rmcp::model::CallToolRequestParams::new("testapp_hello"))
        .await
        .expect("tools/call failed");

    assert_eq!(call.is_error, Some(false));

    let _ = client.cancel().await;
    let _ = server.cancel().await;
}

#[tokio::test]
async fn test_tools_list_and_call_via_mcp_serve_stdio_subcommand() {
    let _ = env_logger::try_init();

    use std::process::Stdio;
    use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
    use tokio::process::Command as TokioCommand;

    struct ChildTransport {
        reader: tokio::process::ChildStdout,
        writer: tokio::process::ChildStdin,
    }

    impl AsyncRead for ChildTransport {
        fn poll_read(
            mut self: std::pin::Pin<&mut Self>,
            cx: &mut std::task::Context<'_>,
            buf: &mut ReadBuf<'_>,
        ) -> std::task::Poll<std::io::Result<()>> {
            std::pin::Pin::new(&mut self.reader).poll_read(cx, buf)
        }
    }

    impl AsyncWrite for ChildTransport {
        fn poll_write(
            mut self: std::pin::Pin<&mut Self>,
            cx: &mut std::task::Context<'_>,
            data: &[u8],
        ) -> std::task::Poll<std::io::Result<usize>> {
            std::pin::Pin::new(&mut self.writer).poll_write(cx, data)
        }

        fn poll_flush(
            mut self: std::pin::Pin<&mut Self>,
            cx: &mut std::task::Context<'_>,
        ) -> std::task::Poll<std::io::Result<()>> {
            std::pin::Pin::new(&mut self.writer).poll_flush(cx)
        }

        fn poll_shutdown(
            mut self: std::pin::Pin<&mut Self>,
            cx: &mut std::task::Context<'_>,
        ) -> std::task::Poll<std::io::Result<()>> {
            std::pin::Pin::new(&mut self.writer).poll_shutdown(cx)
        }
    }

    let server_exe = env!("CARGO_BIN_EXE_cfw_mcp_stdio_test_server");
    let mut child = TokioCommand::new(server_exe)
        .args(["mcp", "serve", "--transport", "stdio"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
        .expect("spawn mcp stdio server");

    let child_stdin = child.stdin.take().expect("child stdin");
    let child_stdout = child.stdout.take().expect("child stdout");
    let transport = ChildTransport {
        reader: child_stdout,
        writer: child_stdin,
    };

    let client = rmcp::serve_client((), transport)
        .await
        .expect("serve_client failed");

    let tools = client
        .peer()
        .list_tools(Default::default())
        .await
        .expect("tools/list failed");

    assert!(
        tools
            .tools
            .iter()
            .any(|t| t.name == "cfw-mcp-stdio-test-server_ping"),
        "expected cfw-mcp-stdio-test-server_ping in tools: {:?}",
        tools.tools
    );

    let call = client
        .peer()
        .call_tool(rmcp::model::CallToolRequestParams::new(
            "cfw-mcp-stdio-test-server_ping",
        ))
        .await
        .expect("tools/call failed");

    assert_eq!(call.is_error, Some(false));

    let _ = client.cancel().await;
    let _ = child.kill().await;
}

/// CF-6: a populated `ResourceRegistry`, handed to the serve path via
/// `AppBuilder::with_mcp_resource_registry`, is actually served end-to-end over
/// the stdio handler path. Drives the real `mcp serve --transport stdio`
/// subprocess and asserts `resources/list` lists the `ui://…` resource and
/// `resources/read` returns its body. This is the regression that the prior
/// wiring (serve entry points never calling `with_resource_registry`) failed.
#[tokio::test]
async fn test_resources_list_and_read_via_mcp_serve_stdio_subcommand() {
    let _ = env_logger::try_init();

    use std::process::Stdio;
    use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
    use tokio::process::Command as TokioCommand;

    struct ChildTransport {
        reader: tokio::process::ChildStdout,
        writer: tokio::process::ChildStdin,
    }

    impl AsyncRead for ChildTransport {
        fn poll_read(
            mut self: std::pin::Pin<&mut Self>,
            cx: &mut std::task::Context<'_>,
            buf: &mut ReadBuf<'_>,
        ) -> std::task::Poll<std::io::Result<()>> {
            std::pin::Pin::new(&mut self.reader).poll_read(cx, buf)
        }
    }

    impl AsyncWrite for ChildTransport {
        fn poll_write(
            mut self: std::pin::Pin<&mut Self>,
            cx: &mut std::task::Context<'_>,
            data: &[u8],
        ) -> std::task::Poll<std::io::Result<usize>> {
            std::pin::Pin::new(&mut self.writer).poll_write(cx, data)
        }

        fn poll_flush(
            mut self: std::pin::Pin<&mut Self>,
            cx: &mut std::task::Context<'_>,
        ) -> std::task::Poll<std::io::Result<()>> {
            std::pin::Pin::new(&mut self.writer).poll_flush(cx)
        }

        fn poll_shutdown(
            mut self: std::pin::Pin<&mut Self>,
            cx: &mut std::task::Context<'_>,
        ) -> std::task::Poll<std::io::Result<()>> {
            std::pin::Pin::new(&mut self.writer).poll_shutdown(cx)
        }
    }

    let server_exe = env!("CARGO_BIN_EXE_cfw_mcp_stdio_test_server");
    let mut child = TokioCommand::new(server_exe)
        .args(["mcp", "serve", "--transport", "stdio"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
        .expect("spawn mcp stdio server");

    let child_stdin = child.stdin.take().expect("child stdin");
    let child_stdout = child.stdout.take().expect("child stdout");
    let transport = ChildTransport {
        reader: child_stdout,
        writer: child_stdin,
    };

    let client = rmcp::serve_client((), transport)
        .await
        .expect("serve_client failed");

    // resources/list must include the registered ui:// resource.
    let listed = client
        .peer()
        .list_resources(Default::default())
        .await
        .expect("resources/list failed");
    assert!(
        listed
            .resources
            .iter()
            .any(|r| r.uri == "ui://cfw-test/index.html"),
        "expected ui://cfw-test/index.html in resources: {:?}",
        listed.resources
    );

    // resources/read must return the registered body.
    let read = client
        .peer()
        .read_resource(rmcp::model::ReadResourceRequestParams::new(
            "ui://cfw-test/index.html",
        ))
        .await
        .expect("resources/read failed");
    let json = serde_json::to_value(&read).unwrap();
    let content = &json["contents"][0];
    assert_eq!(content["uri"], "ui://cfw-test/index.html");
    assert_eq!(content["mimeType"], "text/html");
    assert_eq!(
        content["text"], "<!doctype html><title>cfw-test</title><main>hi</main>",
        "expected served HTML body, got: {json}"
    );

    let _ = client.cancel().await;
    let _ = child.kill().await;
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

/// `AppBuilder::with_mcp_request_authenticator` must wire the hook through to
/// the `mcp serve --transport stdio` path too (parity with HTTP), even though
/// stdio never invokes it. Runs in-process rather than via a subprocess: the
/// test harness's own stdin is already closed/EOF, so the stdio transport
/// returns almost immediately with an I/O error instead of blocking — enough
/// to prove the authenticator-carrying `mcp serve` command actually runs this
/// path end to end (`AppBuilder` → `create_mcp_serve_command_with_deps` →
/// `serve_mcp_stdio_opts_with_resources`) without hanging the test suite.
#[tokio::test]
async fn test_mcp_serve_stdio_with_authenticator_installed_does_not_hang() {
    struct Ctx;
    impl AppContext for Ctx {}

    let mut app = AppBuilder::new()
        .with_version("testapp", "0.1.0")
        .with_mcp_request_authenticator(bearer_authenticator())
        .build(Ctx)
        .unwrap();

    let result = tokio::time::timeout(
        std::time::Duration::from_secs(5),
        app.run_with_args(vec![
            "testapp".to_string(),
            "mcp".to_string(),
            "serve".to_string(),
            "--transport".to_string(),
            "stdio".to_string(),
        ]),
    )
    .await;

    assert!(
        result.is_ok(),
        "mcp serve --transport stdio must not hang when stdin is closed, even with an authenticator installed"
    );
}

// ── Per-request identity seam (T1: MCP request-identity plumbing) ─────────
//
// End-to-end coverage over the real HTTP transport: an
// `AppBuilder::with_mcp_request_authenticator` hook reads the actual
// `Authorization` header off a real `reqwest` HTTP request, and the tool's
// `execute` closure reads the resulting identity back via
// `ctx.request_identity::<T>()`.

/// Stand-in for a downstream product's own identity type (e.g. EntityStore's
/// `SecurityContext`, built from a validated Bearer token). cli-framework
/// never names this type; it only ever sees `Arc<dyn Any + Send + Sync>`.
#[derive(Debug, Clone, PartialEq, Eq)]
struct TestCallerId(String);

fn whoami_command() -> Command {
    Command {
        id: Arc::from("whoami"),
        spec: Arc::new(CommandSpec {
            summary: "report the caller identity established via the MCP request authenticator",
            ..Default::default()
        }),
        validator: None,
        expose_mcp: true,
        expose_chat: true,
        meta: None,
        visibility: None,
        execute: Arc::new(|ctx, _args| {
            Box::pin(async move {
                let who = ctx
                    .request_identity::<TestCallerId>()
                    .map(|id| id.0.clone())
                    .unwrap_or_else(|| "anonymous".to_string());
                ctx.framework_println(&who);
                Ok(())
            })
        }),
    }
}

/// Bearer-token authenticator: `Authorization: Bearer <token>` → `TestCallerId(<token>)`.
/// Missing/malformed header → `None` (anonymous caller — not an error).
fn bearer_authenticator() -> cli_framework::mcp::McpRequestAuthenticator {
    Arc::new(|headers: &http::HeaderMap| {
        let value = headers.get(http::header::AUTHORIZATION)?.to_str().ok()?;
        let token = value.strip_prefix("Bearer ")?;
        Some(Arc::new(TestCallerId(token.to_string())) as Arc<dyn std::any::Any + Send + Sync>)
    })
}

/// Spawns `testapp mcp serve --port <port>` on a background thread, optionally
/// with `bearer_authenticator()` installed, waits for it to answer HTTP, and
/// returns the bound port.
async fn spawn_whoami_server(install_authenticator: bool) -> u16 {
    let port_guard = port_lock().lock().await;
    let port = reserve_port();

    std::thread::spawn(move || {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("tokio runtime");
        rt.block_on(async move {
            struct Ctx;
            impl AppContext for Ctx {}

            let mut builder = AppBuilder::new()
                .with_version("testapp", "0.1.0")
                .register_command(whoami_command())
                .unwrap();
            if install_authenticator {
                builder = builder.with_mcp_request_authenticator(bearer_authenticator());
            }
            let mut app = builder.build(Ctx).unwrap();

            record_server_exit(
                "mcp serve",
                app.run_with_args(vec![
                    "testapp".to_string(),
                    "mcp".to_string(),
                    "serve".to_string(),
                    "--port".to_string(),
                    port.to_string(),
                ])
                .await,
            );
        });
    });

    // Wait here, holding the allocation lock, so the port is bound before any
    // other test is allowed to allocate. The caller runs its own `initialize`
    // to get the session it will use.
    let base_url = format!("http://127.0.0.1:{}", port);
    wait_for_http_server(&reqwest::Client::new(), &base_url).await;
    drop(port_guard);

    port
}

async fn call_whoami(client: &reqwest::Client, base_url: &str, bearer: Option<&str>) -> String {
    let session_id = wait_for_http_server(client, base_url).await;

    let mut req = client
        .post(format!("{}/mcp", base_url))
        .header("Content-Type", "application/json")
        .header("Accept", "application/json, text/event-stream");
    if let Some(ref sid) = session_id {
        req = req.header("Mcp-Session-Id", sid);
    }
    if let Some(token) = bearer {
        req = req.header("Authorization", format!("Bearer {}", token));
    }

    let resp = req
        .json(&serde_json::json!({
            "jsonrpc": "2.0",
            "id": "whoami",
            "method": "tools/call",
            "params": {
                "name": "testapp_whoami",
                "arguments": {}
            }
        }))
        .send()
        .await
        .expect("tools/call request failed");

    assert!(resp.status().is_success(), "status: {}", resp.status());
    let body = resp.text().await.unwrap();
    let json = parse_sse_data(&body);
    json.pointer("/result/content/0/text")
        .and_then(|t| t.as_str())
        .unwrap_or_default()
        .trim()
        .to_string()
}

/// Requirement: "authenticator invoked with request headers; identity visible
/// + correctly downcast in a tool's execute" AND "missing/empty header →
/// None". A real `Authorization: Bearer <token>` header sent over HTTP must
/// reach the authenticator closure and the resulting identity must be
/// readable, correctly typed, inside the tool's `execute`; a request with no
/// such header must yield `None` (not an error). Both assertions share one
/// background server (rather than one each) to limit the number of
/// concurrently spawned HTTP servers this test binary needs.
#[tokio::test]
async fn test_mcp_request_authenticator_identity_visible_and_missing_header_yields_none() {
    let _ = env_logger::try_init();
    let port = spawn_whoami_server(true).await;
    let client = reqwest::Client::new();
    let base_url = format!("http://127.0.0.1:{}", port);

    let who = call_whoami(&client, &base_url, Some("alice-token")).await;
    assert_eq!(who, "alice-token");

    let who = call_whoami(&client, &base_url, None).await;
    assert_eq!(who, "anonymous");
}

/// Requirement: "no authenticator installed → None". Even though the request
/// carries a real bearer token, no hook was installed via
/// `AppBuilder::with_mcp_request_authenticator`, so behavior is unchanged
/// from before the seam existed: the tool never sees an identity.
#[tokio::test]
async fn test_mcp_no_authenticator_installed_yields_none() {
    let _ = env_logger::try_init();
    let port = spawn_whoami_server(false).await;
    let client = reqwest::Client::new();
    let base_url = format!("http://127.0.0.1:{}", port);

    let who = call_whoami(&client, &base_url, Some("alice-token")).await;
    assert_eq!(who, "anonymous");
}

// ── Per-caller tool set (`with_mcp_dynamic_tools`) ────────────────────────
//
// End-to-end coverage over the real HTTP transport, reusing the identity
// harness above: `AppBuilder::with_mcp_dynamic_tools` installs a provider that
// turns the opaque per-request identity into that caller's extra commands, and
// both `tools/list` and `tools/call` consume it. The assertions below are all
// made from outside the server — real Bearer headers in, real JSON-RPC out.

/// A per-caller command that prints one fixed string.
fn echo_command(id: &'static str, text: &'static str) -> Command {
    Command {
        id: Arc::from(id),
        spec: Arc::new(CommandSpec {
            summary: "echo a fixed string",
            ..Default::default()
        }),
        validator: None,
        expose_mcp: true,
        expose_chat: false,
        meta: None,
        visibility: None,
        execute: Arc::new(move |ctx, _args| {
            Box::pin(async move {
                ctx.framework_println(text);
                Ok(())
            })
        }),
    }
}

/// Like [`echo_command`], but with a caller-chosen summary, so two commands
/// published under the *same* tool name stay distinguishable in `tools/list`
/// (`description`) and in `tools/call` (the text it prints).
fn labelled_command(id: &'static str, summary: &'static str, text: &'static str) -> Command {
    let mut cmd = echo_command(id, text);
    cmd.spec = Arc::new(CommandSpec {
        summary,
        ..Default::default()
    });
    cmd
}

/// A per-caller command carrying everything the *static* descriptor path is
/// able to express — a typed argument, opaque `_meta`, and `visibility` tags.
///
/// Its `tools/list` entry is what proves per-caller descriptors are built by
/// the same `command_to_tool_descriptor_full` the static tools use: a
/// hand-rolled descriptor would drop the schema, the `_meta`, or both.
fn rich_command() -> Command {
    use cli_framework::spec::arg_spec::{ArgKind, ArgSpec, ArgValueType, Cardinality};

    Command {
        id: Arc::from("alice_second"),
        spec: Arc::new(CommandSpec {
            summary: "second per-caller tool, with a typed argument",
            args: vec![ArgSpec {
                name: "note",
                kind: ArgKind::Option,
                short: None,
                long: Some("note"),
                value_type: ArgValueType::String,
                cardinality: Cardinality::Optional,
                default: None,
                conflicts_with: vec![],
                requires: vec![],
                help: "a note",
                ..Default::default()
            }],
            ..Default::default()
        }),
        validator: None,
        expose_mcp: true,
        expose_chat: false,
        meta: None,
        visibility: None,
        execute: Arc::new(|ctx, _args| {
            Box::pin(async move {
                ctx.framework_println("alice-second-tool");
                Ok(())
            })
        }),
    }
    .with_meta(serde_json::json!({ "x_tenant": "alice" }))
    .with_visibility(vec!["app".to_string()])
}

/// A per-caller command whose *static* `CommandSpec` declares exactly one
/// argument and a summary nothing should ever advertise, and whose `execute`
/// prints every key it actually received, sorted.
///
/// Both halves matter for the presentation tests: the descriptor it would
/// generate is visibly different from the presentation attached to it, and the
/// printed keys show what `tools/call` really delivered — including a key that
/// appears in neither the static spec nor the presented schema.
fn arg_echo_command(id: &'static str) -> Command {
    use cli_framework::spec::arg_spec::{ArgKind, ArgSpec, ArgValueType, Cardinality};
    use cli_framework::spec::value::ArgValue;
    use std::collections::HashMap;

    Command {
        id: Arc::from(id),
        spec: Arc::new(CommandSpec {
            summary: "STATIC SUMMARY, must not be advertised",
            args: vec![ArgSpec {
                name: "declared",
                kind: ArgKind::Option,
                short: None,
                long: Some("declared"),
                value_type: ArgValueType::String,
                cardinality: Cardinality::Optional,
                default: None,
                conflicts_with: vec![],
                requires: vec![],
                help: "the only statically declared argument",
                ..Default::default()
            }],
            ..Default::default()
        }),
        validator: None,
        expose_mcp: true,
        expose_chat: false,
        meta: None,
        visibility: None,
        execute: Arc::new(|ctx, args: HashMap<String, ArgValue>| {
            let mut seen: Vec<String> = args.iter().map(|(k, v)| format!("{k}={v}")).collect();
            seen.sort();
            Box::pin(async move {
                ctx.framework_println(&seen.join(","));
                Ok(())
            })
        }),
    }
    .with_meta(serde_json::json!({ "x_tenant": "plugin" }))
    .with_visibility(vec!["app".to_string()])
}

/// The runtime-shaped schema a tenant plugin advertises: argument names and
/// help text that exist only as `String`s at request time, and which the
/// `&'static str` `ArgSpec` of [`arg_echo_command`] therefore cannot express.
fn plugin_schema() -> serde_json::Value {
    serde_json::json!({
        "type": "object",
        "properties": {
            "tenant_field": {
                "type": "string",
                "description": "a column name that only exists in the tenant's database"
            }
        },
        "required": ["tenant_field"],
        "additionalProperties": false
    })
}

/// The per-caller tools served to `plugin-token`, exercising every branch of
/// the presentation rules in one provider result:
///
/// 1. `testapp_plugin_report` — a presentation, on a command whose static spec
///    says something else entirely.
/// 2. `testapp_plugin_plain` — the tuple form via `.into()`, i.e. no
///    presentation: the compatibility case.
/// 3. `testapp_plugin_dup` twice — first plain and winning, then presented and
///    dropped, so a presentation cannot win a de-duplication race.
/// 4. `testapp_whoami` — presented *and* colliding with a static tool, so the
///    static-wins rule beats a presentation too.
fn plugin_tools() -> Vec<McpDynamicTool> {
    vec![
        McpDynamicTool {
            name: "testapp_plugin_report".to_string(),
            command: arg_echo_command("plugin_report"),
            presentation: Some(McpToolPresentation {
                description: "Run the tenant's Monthly Report plugin".to_string(),
                input_schema: plugin_schema(),
            }),
        },
        (
            "testapp_plugin_plain".to_string(),
            arg_echo_command("plugin_plain"),
        )
            .into(),
        (
            "testapp_plugin_dup".to_string(),
            labelled_command(
                "plugin_dup_first",
                "FIRST plugin dup, must win",
                "dup-first",
            ),
        )
            .into(),
        McpDynamicTool {
            name: "testapp_plugin_dup".to_string(),
            command: labelled_command("plugin_dup_second", "unused", "dup-second"),
            presentation: Some(McpToolPresentation {
                description: "SECOND plugin dup, presented, must be dropped".to_string(),
                input_schema: serde_json::json!({ "type": "object", "x_dropped": true }),
            }),
        },
        McpDynamicTool {
            name: "testapp_whoami".to_string(),
            command: echo_command("whoami_presented", "PRESENTED-COLLISION"),
            presentation: Some(McpToolPresentation {
                description: "PRESENTED COLLISION, must never be advertised".to_string(),
                input_schema: serde_json::json!({ "type": "object", "x_collision": true }),
            }),
        },
    ]
}

/// Per-caller tool provider used by the tests below.
///
/// - `alice-token` → `testapp_alice_only`, then `testapp_alice_second`
///   (two tools, in that order, so list ordering is observable)
/// - `bob-token`   → `testapp_bob_only`
/// - `dup-token`   → `testapp_dup` **twice**, with different summaries and
///   different output, to exercise the first-pair-wins rule *within* one
///   provider result (a distinct rule from the static collision below)
/// - `plugin-token` → [`plugin_tools`], the `McpToolPresentation` cases
/// - no identity   → `testapp_anonymous_only` (proving the provider ran with
///   `None` rather than being skipped)
///
/// Every caller is additionally offered `testapp_whoami`, which collides with
/// the statically registered command of that name. Static must win: the
/// collision entry must never be dispatched and must never appear twice in
/// `tools/list`.
///
/// `calls` counts awaited invocations, so a test can assert the provider runs
/// once per request and that no result is reused across requests.
fn tenant_provider(calls: Arc<std::sync::atomic::AtomicUsize>) -> McpDynamicToolProvider {
    Arc::new(move |identity| {
        let calls = Arc::clone(&calls);
        let who = identity
            .and_then(|id| id.downcast_ref::<TestCallerId>().map(|c| c.0.clone()))
            .unwrap_or_else(|| "anonymous".to_string());
        Box::pin(async move {
            calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            let mut tools: Vec<McpDynamicTool> = match who.as_str() {
                "alice-token" => vec![
                    (
                        "testapp_alice_only".to_string(),
                        echo_command("alice_only", "alice-tool"),
                    )
                        .into(),
                    ("testapp_alice_second".to_string(), rich_command()).into(),
                ],
                "bob-token" => vec![(
                    "testapp_bob_only".to_string(),
                    echo_command("bob_only", "bob-tool"),
                )
                    .into()],
                // Two pairs under one name: a realistic collision when two
                // tenant plugins declare the same action verb. The first must
                // win on both paths, and the name must appear once.
                "dup-token" => vec![
                    (
                        "testapp_dup".to_string(),
                        labelled_command("dup_first", "FIRST pair, must win", "dup-first"),
                    )
                        .into(),
                    (
                        "testapp_dup".to_string(),
                        labelled_command(
                            "dup_second",
                            "SECOND pair, must be dropped",
                            "dup-second",
                        ),
                    )
                        .into(),
                ],
                "plugin-token" => plugin_tools(),
                _ => vec![(
                    "testapp_anonymous_only".to_string(),
                    echo_command("anonymous_only", "anonymous-tool"),
                )
                    .into()],
            };
            tools.push(
                (
                    "testapp_whoami".to_string(),
                    echo_command("whoami", "DYNAMIC-COLLISION"),
                )
                    .into(),
            );
            tools
        })
    })
}

/// Spawns `testapp mcp serve --port <port>` with [`tenant_provider`] installed
/// and, optionally, the bearer authenticator. Mirrors [`spawn_whoami_server`].
async fn spawn_dynamic_server(
    install_authenticator: bool,
    calls: Arc<std::sync::atomic::AtomicUsize>,
) -> u16 {
    let port_guard = port_lock().lock().await;
    let port = reserve_port();

    std::thread::spawn(move || {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("tokio runtime");
        rt.block_on(async move {
            struct Ctx;
            impl AppContext for Ctx {}

            let mut builder = AppBuilder::new()
                .with_version("testapp", "0.1.0")
                .register_command(whoami_command())
                .unwrap()
                .with_mcp_dynamic_tools(tenant_provider(calls));
            if install_authenticator {
                builder = builder.with_mcp_request_authenticator(bearer_authenticator());
            }
            let mut app = builder.build(Ctx).unwrap();

            record_server_exit(
                "mcp serve (dynamic tools)",
                app.run_with_args(vec![
                    "testapp".to_string(),
                    "mcp".to_string(),
                    "serve".to_string(),
                    "--port".to_string(),
                    port.to_string(),
                ])
                .await,
            );
        });
    });

    let base_url = format!("http://127.0.0.1:{}", port);
    wait_for_http_server(&reqwest::Client::new(), &base_url).await;
    drop(port_guard);

    port
}

/// One JSON-RPC round trip over the real HTTP transport, optionally bearing an
/// `Authorization: Bearer` header. Returns the parsed response envelope, so a
/// caller can inspect `result` or `error`.
async fn mcp_rpc(
    client: &reqwest::Client,
    base_url: &str,
    bearer: Option<&str>,
    body: serde_json::Value,
) -> serde_json::Value {
    let session_id = wait_for_http_server(client, base_url).await;

    let mut req = client
        .post(format!("{}/mcp", base_url))
        .header("Content-Type", "application/json")
        .header("Accept", "application/json, text/event-stream");
    if let Some(ref sid) = session_id {
        req = req.header("Mcp-Session-Id", sid);
    }
    if let Some(token) = bearer {
        req = req.header("Authorization", format!("Bearer {}", token));
    }

    let resp = req.json(&body).send().await.expect("mcp request failed");
    assert!(resp.status().is_success(), "status: {}", resp.status());
    parse_sse_data(&resp.text().await.unwrap())
}

/// The raw `result.tools` array a caller sees from `tools/list`.
async fn list_tools_raw(
    client: &reqwest::Client,
    base_url: &str,
    bearer: Option<&str>,
) -> Vec<serde_json::Value> {
    let json = mcp_rpc(
        client,
        base_url,
        bearer,
        serde_json::json!({"jsonrpc": "2.0", "id": "list", "method": "tools/list"}),
    )
    .await;
    json.pointer("/result/tools")
        .and_then(|t| t.as_array())
        .unwrap_or_else(|| panic!("tools/list returned no tools: {json}"))
        .clone()
}

async fn list_tool_names(
    client: &reqwest::Client,
    base_url: &str,
    bearer: Option<&str>,
) -> Vec<String> {
    list_tools_raw(client, base_url, bearer)
        .await
        .iter()
        .filter_map(|t| t["name"].as_str().map(str::to_string))
        .collect()
}

/// `tools/call` by name, returning the parsed response envelope.
async fn call_tool_rpc(
    client: &reqwest::Client,
    base_url: &str,
    bearer: Option<&str>,
    tool: &str,
) -> serde_json::Value {
    mcp_rpc(
        client,
        base_url,
        bearer,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": "call",
            "method": "tools/call",
            "params": { "name": tool, "arguments": {} }
        }),
    )
    .await
}

/// `tools/call` by name with a caller-supplied `arguments` object, returning
/// the parsed response envelope. [`call_tool_rpc`] is this with `{}`.
async fn call_tool_rpc_with_args(
    client: &reqwest::Client,
    base_url: &str,
    bearer: Option<&str>,
    tool: &str,
    arguments: serde_json::Value,
) -> serde_json::Value {
    mcp_rpc(
        client,
        base_url,
        bearer,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": "call",
            "method": "tools/call",
            "params": { "name": tool, "arguments": arguments }
        }),
    )
    .await
}

/// The text content of a successful `tools/call`, or a panic naming the error.
async fn call_tool_text(
    client: &reqwest::Client,
    base_url: &str,
    bearer: Option<&str>,
    tool: &str,
) -> String {
    let json = call_tool_rpc(client, base_url, bearer, tool).await;
    json.pointer("/result/content/0/text")
        .and_then(|t| t.as_str())
        .unwrap_or_else(|| panic!("tools/call {tool} did not succeed: {json}"))
        .trim()
        .to_string()
}

/// Requirement: two different Bearer identities receive two different
/// `tools/list` results, and the per-caller entries carry the same descriptor
/// shape the static path produces (schema, `_meta`, `visibility`), since both
/// go through `command_to_tool_descriptor_full`.
#[tokio::test]
async fn test_dynamic_tools_two_identities_see_different_tool_lists() {
    let _ = env_logger::try_init();
    let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let port = spawn_dynamic_server(true, Arc::clone(&calls)).await;
    let client = reqwest::Client::new();
    let base_url = format!("http://127.0.0.1:{}", port);

    let alice = list_tool_names(&client, &base_url, Some("alice-token")).await;
    let bob = list_tool_names(&client, &base_url, Some("bob-token")).await;

    assert!(
        alice.contains(&"testapp_alice_only".to_string())
            && alice.contains(&"testapp_alice_second".to_string()),
        "alice must see her own tools: {alice:?}"
    );
    assert!(
        !alice.contains(&"testapp_bob_only".to_string()),
        "alice must not see bob's tools: {alice:?}"
    );
    assert!(
        bob.contains(&"testapp_bob_only".to_string()),
        "bob must see his own tool: {bob:?}"
    );
    assert!(
        !bob.contains(&"testapp_alice_only".to_string())
            && !bob.contains(&"testapp_alice_second".to_string()),
        "bob must not see alice's tools: {bob:?}"
    );

    // The static tool is in both lists — the provider only ever adds.
    assert!(
        alice.contains(&"testapp_whoami".to_string())
            && bob.contains(&"testapp_whoami".to_string()),
        "static tools stay visible to everyone: {alice:?} / {bob:?}"
    );

    // Descriptor parity with the static path: the typed argument reached the
    // generated `inputSchema`, and the opaque `_meta` plus `visibility` tags
    // survived onto the wire exactly as they do for a statically registered
    // command.
    let entries = list_tools_raw(&client, &base_url, Some("alice-token")).await;
    let rich = entries
        .iter()
        .find(|t| t["name"] == "testapp_alice_second")
        .unwrap_or_else(|| panic!("testapp_alice_second missing: {entries:?}"));
    assert_eq!(
        rich["description"], "second per-caller tool, with a typed argument",
        "description must come from the command's own summary: {rich}"
    );
    assert_eq!(
        rich["inputSchema"]["properties"]["note"]["type"], "string",
        "per-caller tools must get the same generated input schema: {rich}"
    );
    assert_eq!(
        rich["_meta"]["x_tenant"], "alice",
        "opaque _meta must pass through for per-caller tools: {rich}"
    );
    assert_eq!(
        rich["_meta"]["visibility"],
        serde_json::json!(["app"]),
        "visibility tags must pass through for per-caller tools: {rich}"
    );
}

/// Requirement: a tool served only to identity A is callable by A and returns
/// `MCP_CMD_NOT_FOUND` for identity B — the list and the dispatch path agree.
#[tokio::test]
async fn test_dynamic_tool_is_callable_only_by_the_identity_it_was_listed_for() {
    let _ = env_logger::try_init();
    let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let port = spawn_dynamic_server(true, Arc::clone(&calls)).await;
    let client = reqwest::Client::new();
    let base_url = format!("http://127.0.0.1:{}", port);

    let text = call_tool_text(
        &client,
        &base_url,
        Some("alice-token"),
        "testapp_alice_only",
    )
    .await;
    assert_eq!(text, "alice-tool");

    let json = call_tool_rpc(&client, &base_url, Some("bob-token"), "testapp_alice_only").await;
    let message = json
        .pointer("/error/message")
        .and_then(|m| m.as_str())
        .unwrap_or_else(|| panic!("bob's call must fail, got: {json}"));
    assert!(
        message.contains("MCP_CMD_NOT_FOUND"),
        "expected MCP_CMD_NOT_FOUND, got: {message}"
    );

    // And the reverse direction, so the test cannot pass by the provider
    // simply returning nothing for one of the two callers.
    let text = call_tool_text(&client, &base_url, Some("bob-token"), "testapp_bob_only").await;
    assert_eq!(text, "bob-tool");
}

/// Requirement: a per-caller name colliding with a statically registered one
/// resolves to the *static* command on dispatch, and `tools/list` emits a
/// single entry for it.
#[tokio::test]
async fn test_static_command_wins_collision_and_is_listed_once() {
    let _ = env_logger::try_init();
    let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let port = spawn_dynamic_server(true, Arc::clone(&calls)).await;
    let client = reqwest::Client::new();
    let base_url = format!("http://127.0.0.1:{}", port);

    // The provider offers its own `testapp_whoami` printing "DYNAMIC-COLLISION".
    // The static command reports the caller identity instead.
    let text = call_tool_text(&client, &base_url, Some("alice-token"), "testapp_whoami").await;
    assert_eq!(
        text, "alice-token",
        "a static tool name must always resolve to the static command"
    );

    let names = list_tool_names(&client, &base_url, Some("alice-token")).await;
    let occurrences = names.iter().filter(|n| *n == "testapp_whoami").count();
    assert_eq!(
        occurrences, 1,
        "a colliding per-caller tool must not be listed a second time: {names:?}"
    );
}

/// Requirement: the ordering of the merged list is the provider's, appended
/// after the static block, and stable across identical requests. The static
/// block's own order is left exactly as `McpToolRegistry::list_tools` produces
/// it — this change deliberately does not start sorting it.
#[tokio::test]
async fn test_dynamic_tools_are_appended_after_static_ones_in_provider_order() {
    let _ = env_logger::try_init();
    let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let port = spawn_dynamic_server(true, Arc::clone(&calls)).await;
    let client = reqwest::Client::new();
    let base_url = format!("http://127.0.0.1:{}", port);

    let names = list_tool_names(&client, &base_url, Some("alice-token")).await;
    let tail: Vec<&String> = names.iter().rev().take(2).rev().collect();
    assert_eq!(
        tail,
        vec!["testapp_alice_only", "testapp_alice_second"],
        "per-caller tools must be appended last, in the order the provider \
         returned them: {names:?}"
    );

    let again = list_tool_names(&client, &base_url, Some("alice-token")).await;
    assert_eq!(
        names, again,
        "two identical requests must produce an identical list"
    );
}

/// Requirement: with a provider installed but no authenticator, the provider
/// is invoked with `None` — not skipped — even though the request carries a
/// real Bearer token that nothing is there to interpret.
#[tokio::test]
async fn test_dynamic_tools_invoked_with_none_identity_when_no_authenticator() {
    let _ = env_logger::try_init();
    let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let port = spawn_dynamic_server(false, Arc::clone(&calls)).await;
    let client = reqwest::Client::new();
    let base_url = format!("http://127.0.0.1:{}", port);

    let names = list_tool_names(&client, &base_url, Some("alice-token")).await;
    assert!(
        names.contains(&"testapp_anonymous_only".to_string()),
        "the provider must run with a None identity, not be skipped: {names:?}"
    );
    assert!(
        !names.contains(&"testapp_alice_only".to_string()),
        "with no authenticator installed there is no identity to key on: {names:?}"
    );

    // The same set is dispatchable, which is the whole point of deriving both
    // from one provider call per request.
    let text = call_tool_text(
        &client,
        &base_url,
        Some("alice-token"),
        "testapp_anonymous_only",
    )
    .await;
    assert_eq!(text, "anonymous-tool");
}

/// Requirement: an authenticated transport with no Bearer header also reaches
/// the provider with `None`.
#[tokio::test]
async fn test_dynamic_tools_invoked_with_none_identity_when_no_bearer_header() {
    let _ = env_logger::try_init();
    let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let port = spawn_dynamic_server(true, Arc::clone(&calls)).await;
    let client = reqwest::Client::new();
    let base_url = format!("http://127.0.0.1:{}", port);

    let names = list_tool_names(&client, &base_url, None).await;
    assert!(
        names.contains(&"testapp_anonymous_only".to_string()),
        "an unauthenticated request must still reach the provider: {names:?}"
    );
}

/// Requirement: the provider runs once per request and nothing is cached
/// across requests with different identities.
#[tokio::test]
async fn test_dynamic_tools_recomputed_per_request_never_cached() {
    let _ = env_logger::try_init();
    let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let port = spawn_dynamic_server(true, Arc::clone(&calls)).await;
    let client = reqwest::Client::new();
    let base_url = format!("http://127.0.0.1:{}", port);
    let count = || calls.load(std::sync::atomic::Ordering::SeqCst);

    let before = count();
    let alice = list_tool_names(&client, &base_url, Some("alice-token")).await;
    assert_eq!(
        count(),
        before + 1,
        "tools/list must call the provider once"
    );

    let bob = list_tool_names(&client, &base_url, Some("bob-token")).await;
    assert_eq!(
        count(),
        before + 2,
        "a second identity must not be served a cached set"
    );
    assert_ne!(alice, bob, "the two identities must get different lists");

    // Asking again as alice recomputes rather than replaying a stored answer.
    let alice_again = list_tool_names(&client, &base_url, Some("alice-token")).await;
    assert_eq!(count(), before + 3, "each request recomputes");
    assert_eq!(alice, alice_again);

    // A `tools/call` that misses the static set consults the provider once...
    call_tool_text(
        &client,
        &base_url,
        Some("alice-token"),
        "testapp_alice_only",
    )
    .await;
    assert_eq!(
        count(),
        before + 4,
        "a static miss must consult the provider for this request"
    );

    // ...and one that hits the static set does not consult it at all.
    call_tool_text(&client, &base_url, Some("alice-token"), "testapp_whoami").await;
    assert_eq!(
        count(),
        before + 4,
        "a static hit must not pay for the provider"
    );
}

/// Regression guard for every existing consumer: with no provider installed,
/// `tools/list` and `tools/call` are exactly what they were before this hook
/// existed — identical for every caller, with no per-caller entries anywhere.
#[tokio::test]
async fn test_no_dynamic_tools_installed_leaves_list_and_call_unchanged() {
    let _ = env_logger::try_init();
    // Authenticator installed but no provider: the strictest form of the
    // guard, since an identity *is* established and still changes nothing.
    let port = spawn_whoami_server(true).await;
    let client = reqwest::Client::new();
    let base_url = format!("http://127.0.0.1:{}", port);

    let alice = list_tools_raw(&client, &base_url, Some("alice-token")).await;
    let bob = list_tools_raw(&client, &base_url, Some("bob-token")).await;
    let anonymous = list_tools_raw(&client, &base_url, None).await;

    assert_eq!(alice, bob, "without a provider every caller sees one list");
    assert_eq!(alice, anonymous, "including the anonymous one");
    assert!(
        alice.iter().any(|t| t["name"] == "testapp_whoami"),
        "the static tool set is still served: {alice:?}"
    );
    assert!(
        !alice
            .iter()
            .any(|t| t["name"].as_str().is_some_and(|n| n.ends_with("_only"))),
        "no per-caller entries may appear: {alice:?}"
    );

    // And dispatch is untouched, including the identity seam it already had.
    assert_eq!(
        call_tool_text(&client, &base_url, Some("alice-token"), "testapp_whoami").await,
        "alice-token"
    );
    let json = call_tool_rpc(&client, &base_url, Some("alice-token"), "testapp_nope").await;
    assert!(
        json.pointer("/error/message")
            .and_then(|m| m.as_str())
            .is_some_and(|m| m.contains("MCP_CMD_NOT_FOUND")),
        "an unknown tool still fails the same way: {json}"
    );
}

// ── Per-caller tool *presentation* (`McpToolPresentation`) ────────────────
//
// A provider that builds a tool from runtime data — a tenant's installed
// plugin, whose description and argument names are database rows — cannot
// express that tool through a `&'static str` `CommandSpec`. It attaches an
// `McpToolPresentation` instead, which `tools/list` advertises in place of the
// derived description and schema. The assertions below are made from outside
// the server, so they are what a real MCP client sees.

/// The raw `tools/list` entry for `tool`, or a panic listing what was there.
async fn list_tool_entry(
    client: &reqwest::Client,
    base_url: &str,
    bearer: Option<&str>,
    tool: &str,
) -> serde_json::Value {
    let entries = list_tools_raw(client, base_url, bearer).await;
    entries
        .iter()
        .find(|t| t["name"] == tool)
        .unwrap_or_else(|| panic!("{tool} missing from tools/list: {entries:?}"))
        .clone()
}

/// Requirement: a presented tool advertises the presentation's `description`
/// and its `inputSchema` *verbatim* — not the command's static summary, and
/// not the schema `build_input_schema` derives from its `CommandSpec`. The
/// schema replaces the derived one wholly: nothing of the static spec's
/// `declared` argument may survive into it.
#[tokio::test]
async fn test_presentation_replaces_description_and_input_schema_in_tools_list() {
    let _ = env_logger::try_init();
    let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let port = spawn_dynamic_server(true, Arc::clone(&calls)).await;
    let client = reqwest::Client::new();
    let base_url = format!("http://127.0.0.1:{}", port);

    let entry = list_tool_entry(
        &client,
        &base_url,
        Some("plugin-token"),
        "testapp_plugin_report",
    )
    .await;

    assert_eq!(
        entry["description"], "Run the tenant's Monthly Report plugin",
        "the presentation's description must be advertised: {entry}"
    );
    assert_ne!(
        entry["description"], "STATIC SUMMARY, must not be advertised",
        "the static summary must not leak into a presented tool: {entry}"
    );
    // Verbatim, field for field — including `required` and
    // `additionalProperties`, which the derived schema never emits.
    assert_eq!(
        entry["inputSchema"],
        plugin_schema(),
        "the presented inputSchema must be emitted exactly as given: {entry}"
    );
    // No merge: the static spec's own argument is nowhere in the advertised
    // schema. A merged schema would have two authors and no one could say
    // which of them a rejected call violated.
    assert!(
        entry["inputSchema"]["properties"]["declared"].is_null(),
        "the derived schema must not be merged into the presentation: {entry}"
    );
}

/// Requirement: a presentation is advertising, never a validation gate. The
/// presented tool is callable, and the arguments reach `execute` unchanged —
/// including `undeclared_extra`, a key in neither the static `CommandSpec` nor
/// the presented schema (whose `additionalProperties` is `false`).
#[tokio::test]
async fn test_presented_tool_is_callable_and_undeclared_arguments_reach_execute() {
    let _ = env_logger::try_init();
    let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let port = spawn_dynamic_server(true, Arc::clone(&calls)).await;
    let client = reqwest::Client::new();
    let base_url = format!("http://127.0.0.1:{}", port);

    let json = call_tool_rpc_with_args(
        &client,
        &base_url,
        Some("plugin-token"),
        "testapp_plugin_report",
        serde_json::json!({
            "tenant_field": "march",
            "undeclared_extra": "kept",
            "declared": "also-kept"
        }),
    )
    .await;

    let text = json
        .pointer("/result/content/0/text")
        .and_then(|t| t.as_str())
        .unwrap_or_else(|| panic!("the presented tool must be callable: {json}"))
        .trim();
    assert_eq!(
        text, "declared=also-kept,tenant_field=march,undeclared_extra=kept",
        "every argument the caller sent must reach execute, declared or not"
    );
}

/// Requirement (compatibility): a per-caller tool built from a `(name,
/// command)` tuple via `.into()` — `presentation: None` — produces exactly the
/// descriptor the pre-change code produced from the static spec. Asserted as
/// whole-value equality against a literal, so any drift in description,
/// schema, `_meta` or `visibility` fails here.
#[tokio::test]
async fn test_tool_without_presentation_keeps_the_pre_change_descriptor() {
    let _ = env_logger::try_init();
    let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let port = spawn_dynamic_server(true, Arc::clone(&calls)).await;
    let client = reqwest::Client::new();
    let base_url = format!("http://127.0.0.1:{}", port);

    let entry = list_tool_entry(
        &client,
        &base_url,
        Some("plugin-token"),
        "testapp_plugin_plain",
    )
    .await;

    assert_eq!(
        entry,
        serde_json::json!({
            "name": "testapp_plugin_plain",
            "description": "STATIC SUMMARY, must not be advertised",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "declared": {
                        "type": "string",
                        "description": "the only statically declared argument"
                    }
                }
            },
            "_meta": { "x_tenant": "plugin", "visibility": ["app"] }
        }),
        "a presentation-less per-caller tool must be described exactly as \
         before presentations existed"
    );
}

/// Requirement: `_meta` and `visibility` come from the `Command` even when a
/// presentation is present — a presentation describes the tool's interface,
/// not the command's passthrough metadata. The presented and the plain tool
/// share one command shape, so their `_meta` must match.
#[tokio::test]
async fn test_presentation_does_not_touch_meta_or_visibility() {
    let _ = env_logger::try_init();
    let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let port = spawn_dynamic_server(true, Arc::clone(&calls)).await;
    let client = reqwest::Client::new();
    let base_url = format!("http://127.0.0.1:{}", port);

    let presented = list_tool_entry(
        &client,
        &base_url,
        Some("plugin-token"),
        "testapp_plugin_report",
    )
    .await;

    assert_eq!(
        presented["_meta"],
        serde_json::json!({ "x_tenant": "plugin", "visibility": ["app"] }),
        "opaque _meta and visibility must still come from the Command: {presented}"
    );
}

/// Requirement: a presentation never wins a de-duplication race. Both drop
/// rules are exercised in one provider result:
///
/// - `testapp_plugin_dup` is returned plain and then presented; the first
///   entry wins and the presented one is dropped whole.
/// - `testapp_whoami` is returned presented and collides with a statically
///   registered command; the static entry wins and is listed once.
///
/// Ordering is unchanged too: the per-caller block follows the static block in
/// the order the provider returned it, with the dropped entries simply absent.
#[tokio::test]
async fn test_presentation_never_wins_a_deduplication_race() {
    let _ = env_logger::try_init();
    let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let port = spawn_dynamic_server(true, Arc::clone(&calls)).await;
    let client = reqwest::Client::new();
    let base_url = format!("http://127.0.0.1:{}", port);

    let entries = list_tools_raw(&client, &base_url, Some("plugin-token")).await;
    let names: Vec<String> = entries
        .iter()
        .filter_map(|t| t["name"].as_str().map(str::to_string))
        .collect();

    // Intra-result: first entry wins, the presented second is not listed.
    assert_eq!(
        names.iter().filter(|n| *n == "testapp_plugin_dup").count(),
        1,
        "a repeated name must be listed once: {names:?}"
    );
    let dup = list_tool_entry(
        &client,
        &base_url,
        Some("plugin-token"),
        "testapp_plugin_dup",
    )
    .await;
    assert_eq!(
        dup["description"], "FIRST plugin dup, must win",
        "the first entry must win; the later presented one is dropped: {dup}"
    );

    // Static collision: the static command wins, presentation or not.
    assert_eq!(
        names.iter().filter(|n| *n == "testapp_whoami").count(),
        1,
        "a colliding name must be listed once: {names:?}"
    );
    let whoami = list_tool_entry(&client, &base_url, Some("plugin-token"), "testapp_whoami").await;
    assert_eq!(
        whoami["description"],
        "report the caller identity established via the MCP request authenticator",
        "the static descriptor must be the one listed: {whoami}"
    );

    // No dropped presentation reached the wire anywhere in the list.
    let wire = serde_json::to_string(&entries).unwrap();
    for dropped in [
        "SECOND plugin dup, presented, must be dropped",
        "PRESENTED COLLISION, must never be advertised",
        "x_dropped",
        "x_collision",
    ] {
        assert!(
            !wire.contains(dropped),
            "a dropped entry must not be advertised at all, found {dropped:?} in {wire}"
        );
    }

    // Ordering is exactly the provider's, minus the drops, appended after the
    // static block.
    let tail: Vec<&String> = names.iter().rev().take(3).rev().collect();
    assert_eq!(
        tail,
        vec![
            "testapp_plugin_report",
            "testapp_plugin_plain",
            "testapp_plugin_dup"
        ],
        "per-caller order must be preserved across presented and plain tools: {names:?}"
    );

    // And the static command still answers its own name.
    let text = call_tool_text(&client, &base_url, Some("plugin-token"), "testapp_whoami").await;
    assert_eq!(
        text, "plugin-token",
        "a presented colliding tool must not become dispatchable"
    );
}

/// Like [`bearer_authenticator`], but counts how many times the framework
/// invokes it, so a test can assert on the *absence* of a call.
fn counting_bearer_authenticator(
    calls: Arc<std::sync::atomic::AtomicUsize>,
) -> cli_framework::mcp::McpRequestAuthenticator {
    Arc::new(move |headers: &http::HeaderMap| {
        calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let value = headers.get(http::header::AUTHORIZATION)?.to_str().ok()?;
        let token = value.strip_prefix("Bearer ")?;
        Some(Arc::new(TestCallerId(token.to_string())) as Arc<dyn std::any::Any + Send + Sync>)
    })
}

/// Spawns `testapp mcp serve --port <port>` with a counting authenticator and,
/// optionally, the per-caller provider. Mirrors [`spawn_dynamic_server`].
async fn spawn_counting_auth_server(
    auth_calls: Arc<std::sync::atomic::AtomicUsize>,
    install_provider: bool,
) -> u16 {
    let port_guard = port_lock().lock().await;
    let port = reserve_port();

    std::thread::spawn(move || {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("tokio runtime");
        rt.block_on(async move {
            struct Ctx;
            impl AppContext for Ctx {}

            let mut builder = AppBuilder::new()
                .with_version("testapp", "0.1.0")
                .register_command(whoami_command())
                .unwrap()
                .with_mcp_request_authenticator(counting_bearer_authenticator(auth_calls));
            if install_provider {
                let provider_calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
                builder = builder.with_mcp_dynamic_tools(tenant_provider(provider_calls));
            }
            let mut app = builder.build(Ctx).unwrap();

            record_server_exit(
                "mcp serve (counting authenticator)",
                app.run_with_args(vec![
                    "testapp".to_string(),
                    "mcp".to_string(),
                    "serve".to_string(),
                    "--port".to_string(),
                    port.to_string(),
                ])
                .await,
            );
        });
    });

    let base_url = format!("http://127.0.0.1:{}", port);
    wait_for_http_server(&reqwest::Client::new(), &base_url).await;
    drop(port_guard);

    port
}

/// The consumer's authenticator is consumer code: it may log, emit metrics or
/// spend a rate-limit budget. Installing no provider must therefore leave
/// `tools/list` exactly as it was before this feature, which means the
/// authenticator is not invoked there at all — its result could not be used.
/// The same authenticator is still invoked on `tools/call` (the seam that
/// already existed), and it *is* invoked on `tools/list` once a provider is
/// installed, so this test fails both on a missing guard and on a guard that
/// disables the seam altogether.
#[tokio::test]
async fn test_tools_list_does_not_invoke_the_authenticator_without_a_provider() {
    let _ = env_logger::try_init();
    let auth_calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let port = spawn_counting_auth_server(Arc::clone(&auth_calls), false).await;
    let client = reqwest::Client::new();
    let base_url = format!("http://127.0.0.1:{}", port);

    // `initialize` happens inside the first helper call; take the baseline
    // after it so only the `tools/list` round trip is measured.
    let _ = list_tools_raw(&client, &base_url, Some("alice-token")).await;
    let before = auth_calls.load(std::sync::atomic::Ordering::SeqCst);
    let _ = list_tools_raw(&client, &base_url, Some("alice-token")).await;
    assert_eq!(
        auth_calls.load(std::sync::atomic::Ordering::SeqCst),
        before,
        "tools/list must not run the authenticator when no provider is installed"
    );

    // The pre-existing dispatch seam is untouched.
    assert_eq!(
        call_tool_text(&client, &base_url, Some("alice-token"), "testapp_whoami").await,
        "alice-token"
    );
    assert!(
        auth_calls.load(std::sync::atomic::Ordering::SeqCst) > before,
        "tools/call must still run the authenticator"
    );

    // With a provider installed, tools/list does run it.
    let auth_calls2 = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let port2 = spawn_counting_auth_server(Arc::clone(&auth_calls2), true).await;
    let base_url2 = format!("http://127.0.0.1:{}", port2);
    let _ = list_tools_raw(&client, &base_url2, Some("alice-token")).await;
    let before2 = auth_calls2.load(std::sync::atomic::Ordering::SeqCst);
    let _ = list_tools_raw(&client, &base_url2, Some("alice-token")).await;
    assert!(
        auth_calls2.load(std::sync::atomic::Ordering::SeqCst) > before2,
        "tools/list must run the authenticator when a provider is installed"
    );
}

/// The other half of the dedup rule: a name repeated *within one provider
/// result*. Static dedup cannot catch this, so it is asserted separately —
/// two tenant plugins declaring the same action verb is a realistic collision,
/// and a `tools/list` carrying two tools with one name is a protocol problem
/// for clients, not an aesthetic one. First pair wins, on both paths.
#[tokio::test]
async fn test_provider_returning_one_name_twice_lists_and_dispatches_the_first() {
    let _ = env_logger::try_init();
    let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let port = spawn_dynamic_server(true, Arc::clone(&calls)).await;
    let client = reqwest::Client::new();
    let base_url = format!("http://127.0.0.1:{}", port);

    let tools = list_tools_raw(&client, &base_url, Some("dup-token")).await;
    let dups: Vec<&serde_json::Value> = tools
        .iter()
        .filter(|t| t["name"] == "testapp_dup")
        .collect();
    assert_eq!(
        dups.len(),
        1,
        "a name the provider returned twice must be described exactly once: {tools:?}"
    );
    assert_eq!(
        dups[0]["description"], "FIRST pair, must win",
        "the entry kept must be the first pair, not the last: {:?}",
        dups[0]
    );

    // And dispatch agrees with the list — the whole point of one hook feeding
    // both paths.
    assert_eq!(
        call_tool_text(&client, &base_url, Some("dup-token"), "testapp_dup").await,
        "dup-first"
    );
}

/// Mirrors `test_mcp_serve_stdio_with_authenticator_installed_does_not_hang`
/// for the new hook: `mcp serve --transport stdio` must still exit on closed
/// stdin with a per-caller provider installed. Under stdio there is no HTTP
/// request, so the provider would be invoked with a `None` identity; what is
/// verified here is that installing one neither breaks the stdio wiring nor
/// blocks.
#[tokio::test]
async fn test_mcp_serve_stdio_with_dynamic_tools_installed_does_not_hang() {
    struct Ctx;
    impl AppContext for Ctx {}

    let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let mut app = AppBuilder::new()
        .with_version("testapp", "0.1.0")
        .with_mcp_dynamic_tools(tenant_provider(calls))
        .build(Ctx)
        .unwrap();

    let result = tokio::time::timeout(
        std::time::Duration::from_secs(5),
        app.run_with_args(vec![
            "testapp".to_string(),
            "mcp".to_string(),
            "serve".to_string(),
            "--transport".to_string(),
            "stdio".to_string(),
        ]),
    )
    .await;

    assert!(
        result.is_ok(),
        "mcp serve --transport stdio must not hang when stdin is closed, even with a per-caller tool provider installed"
    );
}

/// `AppBuilder::with_mcp_http_listener`: `mcp serve` serves on the caller's
/// listener and does not bind `--host`/`--port` itself. `--port 1` would fail
/// to bind for an unprivileged test process, so a served request proves the
/// flag was not consulted. The listener is bound on `:0` and never released,
/// so this test needs neither `reserve_port` nor the port lock.
#[tokio::test]
async fn test_mcp_serve_on_caller_bound_listener() {
    let _ = env_logger::try_init();

    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind 127.0.0.1:0");
    let port = listener.local_addr().expect("local_addr").port();

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
                .register_command(echo_command("widget", "widget ran"))
                .unwrap()
                .with_mcp_http_listener(listener)
                .build(Ctx)
                .unwrap();

            record_server_exit(
                "mcp serve on caller listener",
                app.run_with_args(vec![
                    "testapp".to_string(),
                    "mcp".to_string(),
                    "serve".to_string(),
                    "--port".to_string(),
                    "1".to_string(),
                ])
                .await,
            );
        });
    });

    let client = reqwest::Client::new();
    let base_url = format!("http://127.0.0.1:{}", port);
    wait_for_http_server(&client, &base_url).await;

    let names = list_tool_names(&client, &base_url, None).await;
    assert!(
        names.iter().any(|n| n == "testapp_widget"),
        "testapp_widget not found in tools: {names:?}"
    );
}
