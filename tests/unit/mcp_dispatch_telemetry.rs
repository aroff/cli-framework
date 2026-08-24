//! Telemetry assertions for MCP dispatch, in a **dedicated test binary**.
//!
//! This lives apart from `mcp_dispatch.rs` on purpose. `tracing` keeps a
//! process-wide max-level hint that is recomputed as subscribers come and go,
//! and `opentelemetry`'s tracer provider is likewise process-global (every
//! `init_*` path calls `global::set_tracer_provider`, and `TelemetryGuard::drop`
//! shuts the provider down). Run alongside sibling tests in the same binary,
//! those globals are mutated by other threads while this test is building and
//! closing its span, and the `cli.command` span is intermittently never
//! recorded.
//!
//! Measured on the shared binary: **2 failures in 8 runs**, and the same test
//! filtered to run alone in its process passed **6 for 6**. Cargo gives each
//! `[[test]]` target its own process, so isolating this one removes the
//! interference without serialising unrelated tests or changing production
//! behaviour.
//!
//! If you add another telemetry-asserting MCP test, put it here rather than in
//! `mcp_dispatch.rs` — and keep in mind that tests *within* this file still
//! share the same globals with each other.

#![cfg(feature = "telemetry")]

use cli_framework::command::{Command, CommandRegistry};
use cli_framework::mcp::{dispatch_tool_call, McpToolRegistry, McpTransportKind};
use cli_framework::spec::command_tree::CommandSpec;
use cli_framework::spec::value::ArgValue;
use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

fn noop_execute() -> Arc<
    dyn for<'a> Fn(
            &'a mut dyn cli_framework::app::AppContext,
            HashMap<String, ArgValue>,
        ) -> Pin<Box<dyn Future<Output = anyhow::Result<()>> + Send + 'a>>
        + Send
        + Sync,
> {
    Arc::new(|_ctx, _args| Box::pin(async { Ok(()) }))
}

fn make_cmd(id: &'static str) -> Command {
    Command {
        id: Arc::from(id),
        spec: Arc::new(CommandSpec {
            summary: "test command",
            ..Default::default()
        }),
        validator: None,
        expose_mcp: false,
        expose_chat: true,
        meta: None,
        visibility: None,
        execute: noop_execute(),
    }
}

// ── Gap 2: dispatch_tool_call emits a cli.command span with surface=mcp ───

#[tokio::test]
async fn dispatch_tool_call_emits_mcp_surface_span() {
    use cli_framework::telemetry::init::init_with_exporter;
    use opentelemetry_sdk::error::OTelSdkResult;
    use opentelemetry_sdk::trace::{SpanData, SpanExporter};
    use std::sync::Mutex;
    use tracing_subscriber::prelude::*;

    #[derive(Clone, Default, Debug)]
    struct SpanSink(Arc<Mutex<Vec<SpanData>>>);
    impl SpanExporter for SpanSink {
        async fn export(&self, batch: Vec<SpanData>) -> OTelSdkResult {
            self.0.lock().unwrap().extend(batch);
            Ok(())
        }
    }

    let sink = SpanSink::default();
    let (tel, guard) = init_with_exporter(sink.clone(), "test-mcp-span");

    let tracer = guard.tracer("cli-framework");
    let otel_layer = tracing_opentelemetry::layer().with_tracer(tracer);
    let subscriber = tracing_subscriber::registry().with(otel_layer);

    let cmd = make_cmd("hello");
    let mut reg = CommandRegistry::new();
    reg.register(cmd);
    let tool_registry =
        Arc::new(McpToolRegistry::from_command_registry(&reg, "myapp").with_telemetry(tel));

    // set_default installs the subscriber for this thread's scope without
    // requiring a sync closure, so we can await dispatch_tool_call directly.
    // Safe here because this binary runs only this test.
    let _sub_guard = tracing::subscriber::set_default(subscriber);
    let _ = dispatch_tool_call(&tool_registry, "myapp_hello", None, McpTransportKind::Http).await;

    guard.flush();

    let spans = sink.0.lock().unwrap().clone();
    let mcp_span = spans
        .iter()
        .find(|s| s.name.as_ref() == "cli.command")
        .expect("expected a cli.command span — dispatch_tool_call must emit one");

    let surface = mcp_span
        .attributes
        .iter()
        .find(|kv| kv.key.as_str() == "cli.invocation.surface")
        .map(|kv| kv.value.to_string());

    assert_eq!(
        surface.as_deref(),
        Some("mcp"),
        "cli.invocation.surface must be 'mcp', got: {:?}",
        surface
    );
}
