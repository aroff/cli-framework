use cli_framework::app::{AppBuilder, AppContext, RequestIdentityExt};
use cli_framework::command::{Command, CommandRegistry};
use cli_framework::mcp::{
    serve_mcp_with_gate, CliFrameworkHandler, McpServerArgs, McpToolExportPolicy, McpToolRegistry,
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
