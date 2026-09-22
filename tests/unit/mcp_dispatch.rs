use cli_framework::app::{AppBuilder, AppContext, RequestIdentityExt};
use cli_framework::command::{Command, CommandRegistry};
use cli_framework::mcp::resources::ResourceRegistry;
use cli_framework::mcp::{
    dispatch_tool_call, dispatch_tool_call_spawned, dispatch_tool_call_with_identity,
    serve_mcp_stdio_opts_with_resources, BannerSettings, McpToolRegistry, McpTransportKind,
};
use cli_framework::security::command_risk::CommandRiskTier;
use cli_framework::security::gate::{ExecutionGate, GateError};
use cli_framework::spec::arg_spec::{ArgKind, ArgSpec, ArgValueType, Cardinality};
use cli_framework::spec::command_tree::{CommandPath, CommandSpec};
use cli_framework::spec::value::ArgValue;
use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

struct DummyCtx;
impl AppContext for DummyCtx {}

/// Stage 2 requirement: `mcp serve` is auto-registered after `build()` when `mcp-server` is on.
#[cfg(feature = "mcp-server")]
#[test]
fn mcp_serve_registered_after_build() {
    let app = AppBuilder::new()
        .with_version("testapp", "0.1.0")
        .build(DummyCtx)
        .unwrap();

    let path = CommandPath::new(&["mcp", "serve"]).unwrap();
    let found = app.command_registry().resolve(&path).is_some();
    assert!(found, "mcp/serve not registered in registry after build()");
}

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

fn failing_execute() -> Arc<
    dyn for<'a> Fn(
            &'a mut dyn cli_framework::app::AppContext,
            HashMap<String, ArgValue>,
        ) -> Pin<Box<dyn Future<Output = anyhow::Result<()>> + Send + 'a>>
        + Send
        + Sync,
> {
    Arc::new(|_ctx, _args| Box::pin(async { Err(anyhow::anyhow!("command execution failed")) }))
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

fn make_registry_with_cmd(_id: &'static str, cmd: Command) -> McpToolRegistry {
    let mut registry = CommandRegistry::new();
    registry.register(cmd);
    McpToolRegistry::from_command_registry(&registry, "myapp")
}

#[tokio::test]
async fn test_tool_call_success() {
    let cmd = make_cmd("hello");
    let tool_registry = make_registry_with_cmd("hello", cmd);

    let result =
        dispatch_tool_call(&tool_registry, "myapp_hello", None, McpTransportKind::Http).await;
    assert!(result.is_ok());
    let call_result = result.unwrap();
    assert_eq!(call_result.is_error, Some(false));
}

#[tokio::test]
async fn test_tool_call_cmd_not_found() {
    let tool_registry = McpToolRegistry::from_command_registry(&CommandRegistry::new(), "myapp");

    let result = dispatch_tool_call(
        &tool_registry,
        "myapp_nonexistent",
        None,
        McpTransportKind::Http,
    )
    .await;
    assert!(result.is_err());
    let err = result.unwrap_err();
    assert!(
        err.message.starts_with("MCP_CMD_NOT_FOUND:"),
        "got: {}",
        err.message
    );
}

#[tokio::test]
async fn test_tool_call_arg_validation_failed() {
    let spec = CommandSpec {
        args: vec![ArgSpec {
            name: "required-arg",
            kind: ArgKind::Option,
            short: None,
            long: None,
            value_type: ArgValueType::String,
            cardinality: Cardinality::Required,
            default: None,
            conflicts_with: vec![],
            requires: vec![],
            help: "A required argument",
            ..Default::default()
        }],
        ..Default::default()
    };
    let cmd = Command {
        id: Arc::from("test-cmd"),
        spec: Arc::new(spec),
        validator: None,
        expose_mcp: false,
        expose_chat: true,
        meta: None,
        visibility: None,
        execute: noop_execute(),
    };

    let mut registry = CommandRegistry::new();
    registry.register(cmd);
    let tool_registry = McpToolRegistry::from_command_registry(&registry, "myapp");

    // Call without required arg
    let result = dispatch_tool_call(
        &tool_registry,
        "myapp_test-cmd",
        None,
        McpTransportKind::Http,
    )
    .await;
    assert!(result.is_err());
    let err = result.unwrap_err();
    assert!(
        err.message.starts_with("MCP_ARG_VALIDATION_FAILED:"),
        "got: {}",
        err.message
    );
}

#[tokio::test]
async fn test_tool_call_execution_failed() {
    let cmd = Command {
        id: Arc::from("fail-cmd"),
        spec: Arc::new(CommandSpec {
            summary: "failing command",
            ..Default::default()
        }),
        validator: None,
        expose_mcp: false,
        expose_chat: true,
        meta: None,
        visibility: None,
        execute: failing_execute(),
    };

    let tool_registry = make_registry_with_cmd("fail-cmd", cmd);

    let result = dispatch_tool_call(
        &tool_registry,
        "myapp_fail-cmd",
        None,
        McpTransportKind::Http,
    )
    .await;
    assert!(result.is_err());
    let err = result.unwrap_err();
    assert!(
        err.message.starts_with("MCP_EXECUTION_FAILED:"),
        "got: {}",
        err.message
    );
}

#[tokio::test]
async fn test_tool_call_internal_error() {
    // Verify that a panicking execute closure produces MCP_INTERNAL_ERROR.
    // dispatch_tool_call_spawned runs the call in a tokio::spawn and maps
    // JoinError (panic) → MCP_INTERNAL_ERROR (AC-E-INTERNAL, §4.7).
    let panicking_cmd = Command {
        id: Arc::from("panic-cmd"),
        spec: Arc::new(CommandSpec {
            summary: "Panicking command",
            ..Default::default()
        }),
        validator: None,
        expose_mcp: false,
        expose_chat: true,
        meta: None,
        visibility: None,
        execute: Arc::new(|_ctx, _args: HashMap<String, ArgValue>| {
            Box::pin(async move {
                panic!("intentional panic for MCP_INTERNAL_ERROR test");
                #[allow(unreachable_code)]
                Ok(())
            })
        }),
    };

    let mut registry = CommandRegistry::new();
    registry.register(panicking_cmd);
    let tool_registry = Arc::new(McpToolRegistry::from_command_registry(&registry, "myapp"));

    let result = dispatch_tool_call_spawned(
        tool_registry,
        "myapp_panic-cmd".to_string(),
        None,
        McpTransportKind::Http,
    )
    .await;
    assert!(result.is_err());
    let err = result.unwrap_err();
    assert!(
        err.message.starts_with("MCP_INTERNAL_ERROR:"),
        "expected MCP_INTERNAL_ERROR, got: {}",
        err.message
    );
}

#[derive(Debug)]
struct DenyGate;

#[async_trait::async_trait]
impl ExecutionGate for DenyGate {
    async fn before_execute(
        &self,
        _cmd: &Command,
        _args: &HashMap<String, ArgValue>,
        _tier: CommandRiskTier,
    ) -> Result<(), GateError> {
        Err(GateError::Denied {
            reason: "blocked by test gate".to_string(),
        })
    }
}

#[derive(Debug)]
struct FailGate;

#[async_trait::async_trait]
impl ExecutionGate for FailGate {
    async fn before_execute(
        &self,
        _cmd: &Command,
        _args: &HashMap<String, ArgValue>,
        _tier: CommandRiskTier,
    ) -> Result<(), GateError> {
        Err(GateError::Failed {
            reason: "gate crashed".to_string(),
        })
    }
}

#[tokio::test]
async fn test_gate_denied_maps_to_mcp_tool_denied() {
    let cmd = make_cmd("hello");
    let mut registry = CommandRegistry::new();
    registry.register(cmd);
    let tool_registry =
        McpToolRegistry::from_command_registry(&registry, "myapp").with_gate(Arc::new(DenyGate));

    let err = dispatch_tool_call(&tool_registry, "myapp_hello", None, McpTransportKind::Http)
        .await
        .unwrap_err();

    assert_eq!(err.code, rmcp::model::ErrorCode(-32005));
    assert!(
        err.message.starts_with("MCP_TOOL_DENIED:"),
        "got: {}",
        err.message
    );
}

#[tokio::test]
async fn test_gate_failed_maps_to_mcp_tool_gate_failed() {
    let cmd = make_cmd("hello");
    let mut registry = CommandRegistry::new();
    registry.register(cmd);
    let tool_registry =
        McpToolRegistry::from_command_registry(&registry, "myapp").with_gate(Arc::new(FailGate));

    let err = dispatch_tool_call(&tool_registry, "myapp_hello", None, McpTransportKind::Http)
        .await
        .unwrap_err();

    assert_eq!(err.code, rmcp::model::ErrorCode(-32006));
    assert!(
        err.message.starts_with("MCP_TOOL_GATE_FAILED:"),
        "got: {}",
        err.message
    );
}

// ── Per-request identity seam (T1: MCP request-identity plumbing) ─────────

/// A stand-in for a downstream product's own identity type (e.g. EntityStore's
/// `SecurityContext`). cli-framework never names this type — it's entirely
/// opaque to the framework, only known to the host's authenticator closure
/// and the tool's own `execute` closure.
#[derive(Debug, Clone, PartialEq, Eq)]
struct CallerId(String);

/// A command whose `execute` reads back the per-request identity via the
/// blanket `RequestIdentityExt::request_identity` and reports it as text —
/// exactly the mechanism a downstream product's own MCP tools would use.
fn whoami_cmd() -> Command {
    Command {
        id: Arc::from("whoami"),
        spec: Arc::new(CommandSpec {
            summary: "report the caller identity, if any",
            ..Default::default()
        }),
        validator: None,
        expose_mcp: false,
        expose_chat: true,
        meta: None,
        visibility: None,
        execute: Arc::new(|ctx, _args: HashMap<String, ArgValue>| {
            Box::pin(async move {
                let who = ctx
                    .request_identity::<CallerId>()
                    .map(|id| id.0.clone())
                    .unwrap_or_else(|| "anonymous".to_string());
                ctx.framework_println(&who);
                Ok(())
            })
        }),
    }
}

fn call_result_text(result: &rmcp::model::CallToolResult) -> String {
    match &result.content[0] {
        rmcp::model::ContentBlock::Text(t) => t.text.clone(),
        other => panic!("expected text content, got {other:?}"),
    }
}

#[tokio::test]
async fn identity_is_visible_and_correctly_downcast_in_tool_execute() {
    let tool_registry = make_registry_with_cmd("whoami", whoami_cmd());
    let identity: Arc<dyn std::any::Any + Send + Sync> = Arc::new(CallerId("alice".to_string()));

    let result = dispatch_tool_call_with_identity(
        &tool_registry,
        "myapp_whoami",
        None,
        McpTransportKind::Http,
        Some(identity),
    )
    .await
    .expect("dispatch ok");

    assert_eq!(call_result_text(&result).trim(), "alice");
}

#[tokio::test]
async fn no_identity_argument_yields_none_in_tool_execute() {
    let tool_registry = make_registry_with_cmd("whoami", whoami_cmd());

    let result = dispatch_tool_call_with_identity(
        &tool_registry,
        "myapp_whoami",
        None,
        McpTransportKind::Http,
        None,
    )
    .await
    .expect("dispatch ok");

    assert_eq!(call_result_text(&result).trim(), "anonymous");
}

/// `dispatch_tool_call` (the pre-existing, non-identity entry point still
/// used by every caller that hasn't opted in) must keep behaving exactly as
/// before: the tool sees no identity at all.
#[tokio::test]
async fn dispatch_tool_call_without_identity_param_stays_none() {
    let tool_registry = make_registry_with_cmd("whoami", whoami_cmd());

    let result = dispatch_tool_call(&tool_registry, "myapp_whoami", None, McpTransportKind::Http)
        .await
        .expect("dispatch ok");

    assert_eq!(call_result_text(&result).trim(), "anonymous");
}

#[tokio::test]
async fn wrong_type_downcast_yields_none() {
    let tool_registry = make_registry_with_cmd("whoami", whoami_cmd());
    // Stash a value of a type the tool never asks for (i32, not CallerId).
    let identity: Arc<dyn std::any::Any + Send + Sync> = Arc::new(42i32);

    let result = dispatch_tool_call_with_identity(
        &tool_registry,
        "myapp_whoami",
        None,
        McpTransportKind::Http,
        Some(identity),
    )
    .await
    .expect("dispatch ok");

    assert_eq!(
        call_result_text(&result).trim(),
        "anonymous",
        "a stashed identity of the wrong type must downcast to None, not panic or leak"
    );
}

#[tokio::test]
async fn spawned_identity_variant_threads_identity_through() {
    let tool_registry = Arc::new(make_registry_with_cmd("whoami", whoami_cmd()));
    let identity: Arc<dyn std::any::Any + Send + Sync> = Arc::new(CallerId("carol".to_string()));

    let result = cli_framework::mcp::dispatch_tool_call_spawned_with_identity(
        tool_registry,
        "myapp_whoami".to_string(),
        None,
        McpTransportKind::Http,
        Some(identity),
    )
    .await
    .expect("dispatch ok");

    assert_eq!(call_result_text(&result).trim(), "carol");
}

/// `McpToolRegistry::with_request_authenticator` is opt-in: installing one
/// changes nothing about calls made through the non-identity dispatch path
/// (`dispatch_tool_call`), since only the HTTP transport (`call_tool`) ever
/// extracts headers and invokes the hook.
#[tokio::test]
async fn installed_authenticator_does_not_affect_direct_dispatch_calls() {
    let mut registry = CommandRegistry::new();
    registry.register(whoami_cmd());
    let tool_registry = McpToolRegistry::from_command_registry(&registry, "myapp")
        .with_request_authenticator(Arc::new(|_headers: &http::HeaderMap| {
            Some(Arc::new(CallerId("should-not-appear".to_string()))
                as Arc<dyn std::any::Any + Send + Sync>)
        }));

    let result = dispatch_tool_call(&tool_registry, "myapp_whoami", None, McpTransportKind::Http)
        .await
        .expect("dispatch ok");

    assert_eq!(
        call_result_text(&result).trim(),
        "anonymous",
        "dispatch_tool_call (no identity arg) must never invoke the authenticator"
    );
}

/// The stdio serve entry point stores an installed authenticator on the tool
/// registry (for parity with the HTTP entry point / `AppBuilder` wiring), even
/// though it can never fire under stdio. This exercises that storage path
/// in-process: the test harness's own stdin is already closed/EOF, so
/// `serve_server` returns near-instantly with an I/O error rather than
/// blocking — enough to drive every line of the function body without
/// spawning a real stdio subprocess.
#[tokio::test]
async fn serve_mcp_stdio_stores_installed_authenticator_without_blocking() {
    let registry = Arc::new(CommandRegistry::new());
    let authenticator: cli_framework::mcp::McpRequestAuthenticator =
        Arc::new(|_headers: &http::HeaderMap| None);

    let result = tokio::time::timeout(
        std::time::Duration::from_secs(5),
        serve_mcp_stdio_opts_with_resources(
            registry,
            "testapp",
            cli_framework::security::CommandRiskPolicy::default(),
            cli_framework::mcp::McpToolExportPolicy::AllCommands,
            None,
            Arc::new(ResourceRegistry::new()),
            BannerSettings::from_env(),
            None,
            Some(authenticator),
            None,
        ),
    )
    .await;

    assert!(
        result.is_ok(),
        "serve_mcp_stdio_opts_with_resources must not hang when stdin is closed"
    );
}

/// `serve_mcp_stdio_opts` (the resource-less convenience wrapper) must thread
/// its own `None` `request_authenticator` default through to
/// `serve_mcp_stdio_opts_with_resources` without altering behavior.
#[tokio::test]
async fn serve_mcp_stdio_opts_wrapper_does_not_hang() {
    let registry = Arc::new(CommandRegistry::new());

    let result = tokio::time::timeout(
        std::time::Duration::from_secs(5),
        cli_framework::mcp::serve_mcp_stdio_opts(
            registry,
            "testapp",
            cli_framework::security::CommandRiskPolicy::default(),
            cli_framework::mcp::McpToolExportPolicy::AllCommands,
            None,
            BannerSettings::from_env(),
        ),
    )
    .await;

    assert!(
        result.is_ok(),
        "serve_mcp_stdio_opts must not hang when stdin is closed"
    );
}

// ── Per-caller tool set (`McpToolRegistry::with_dynamic_tools`) ───────────
//
// The HTTP half of this hook is covered end to end in
// `tests/integration/mcp_http.rs`. What is covered here is the transport-free
// half: the registry API itself, and the stdio-shaped path where no HTTP
// request — and therefore no identity — can ever exist.

fn tenant_tool(text: &'static str) -> Command {
    Command {
        id: Arc::from("tenant"),
        spec: Arc::new(CommandSpec {
            summary: "a per-caller tool",
            ..Default::default()
        }),
        validator: None,
        expose_mcp: true,
        expose_chat: false,
        meta: None,
        visibility: None,
        execute: Arc::new(move |ctx, _args: HashMap<String, ArgValue>| {
            Box::pin(async move {
                ctx.framework_println(text);
                Ok(())
            })
        }),
    }
}

/// Serves `myapp_tenant` to everyone, and offers a colliding `myapp_whoami`
/// that must always lose to the statically registered command of that name.
fn always_provider() -> cli_framework::mcp::McpDynamicToolProvider {
    Arc::new(|_identity| {
        Box::pin(async move {
            vec![
                ("myapp_tenant".to_string(), tenant_tool("tenant-tool")).into(),
                ("myapp_whoami".to_string(), tenant_tool("DYNAMIC-COLLISION")).into(),
            ]
        })
    })
}

/// With no provider installed, `list_tools_for_identity` is `list_tools()` —
/// same entries, same order — for any identity. This is the API-level form of
/// the regression guard every existing consumer relies on.
///
/// Compared as serialized descriptors, not just names: `McpToolDescriptor`
/// carries the description, the input schema, `_meta` and `visibility`, and
/// this is the cheapest seam at which all four can be held byte-identical to
/// the static path at once.
#[tokio::test]
async fn list_tools_for_identity_without_provider_equals_static_list() {
    let mut registry = CommandRegistry::new();
    registry.register(whoami_cmd());
    // Several commands, not one: with a single tool every ordering is the
    // same ordering, and "ordering included" would assert nothing.
    registry.register(make_cmd("alpha"));
    registry.register(make_cmd("beta"));
    registry.register(make_cmd("gamma"));
    let tool_registry = McpToolRegistry::from_command_registry(&registry, "myapp");
    assert!(
        tool_registry.list_tools().len() > 1,
        "this test only constrains ordering if there is an order to get wrong"
    );

    let static_json = serde_json::to_string(&tool_registry.list_tools()).unwrap();
    let identity: Arc<dyn std::any::Any + Send + Sync> = Arc::new(CallerId("alice".to_string()));

    for who in [None, Some(identity)] {
        let json =
            serde_json::to_string(&tool_registry.list_tools_for_identity(who).await).unwrap();
        assert_eq!(
            json, static_json,
            "without a provider the listing must not vary by identity, \
             in any field or any order"
        );
    }
}

/// With a provider installed, the merged list still *starts* with the static
/// block, byte for byte and in `list_tools()` order; only the per-caller block
/// is new. The HTTP ordering test pins the tail (the provider's own order);
/// this pins the head, which is the half every existing consumer already
/// depends on.
#[tokio::test]
async fn list_tools_for_identity_keeps_the_static_block_verbatim_at_the_front() {
    let mut registry = CommandRegistry::new();
    registry.register(whoami_cmd());
    registry.register(make_cmd("alpha"));
    registry.register(make_cmd("beta"));
    registry.register(make_cmd("gamma"));
    // One registry, not two: `list_tools()` iterates a `HashMap`, whose order
    // belongs to that particular map instance. Comparing across instances
    // would be comparing two unrelated orders.
    let tool_registry = McpToolRegistry::from_command_registry(&registry, "myapp");
    let static_tools = tool_registry.list_tools();
    assert!(static_tools.len() > 1);

    let tool_registry = tool_registry.with_dynamic_tools(always_provider());
    let merged = tool_registry.list_tools_for_identity(None).await;

    assert_eq!(
        serde_json::to_string(&merged[..static_tools.len()]).unwrap(),
        serde_json::to_string(&static_tools).unwrap(),
        "the static block must be emitted first, unchanged and unreordered"
    );
    assert_eq!(
        merged[static_tools.len()..]
            .iter()
            .map(|d| d.name.clone())
            .collect::<Vec<_>>(),
        vec!["myapp_tenant".to_string()],
        "only the per-caller block may follow it (myapp_whoami collides and is dropped)"
    );
}

/// The tuple conversion is the ergonomic escape hatch for providers that do
/// not need a presentation: it must produce `presentation: None`, which is the
/// value that routes the descriptor through the static path.
#[test]
fn tuple_converts_into_a_dynamic_tool_without_a_presentation() {
    let tool: cli_framework::mcp::McpDynamicTool =
        ("myapp_tenant".to_string(), tenant_tool("tenant-tool")).into();
    assert_eq!(tool.name, "myapp_tenant");
    assert_eq!(&*tool.command.id, "tenant");
    assert!(
        tool.presentation.is_none(),
        "the tuple form must never invent a presentation"
    );
}

/// A presentation replaces the derived `description` and `inputSchema` and
/// leaves `_meta` / `visibility` alone. Asserted directly on the descriptor
/// builder, because that is the one place the substitution happens and the
/// HTTP seam can only observe it through the wire format.
#[test]
fn presented_descriptor_replaces_description_and_schema_only() {
    use cli_framework::mcp::{schema::command_to_tool_descriptor_presented, McpToolPresentation};

    let cmd = tenant_tool("tenant-tool")
        .with_meta(serde_json::json!({ "x_tenant": "acme" }))
        .with_visibility(vec!["app".to_string()]);
    let presentation = McpToolPresentation {
        description: "runtime description".to_string(),
        input_schema: serde_json::json!({
            "type": "object",
            "properties": { "runtime_field": { "type": "string" } },
            "required": ["runtime_field"]
        }),
    };

    let presented = command_to_tool_descriptor_presented("myapp_tenant", &cmd, &presentation);
    let derived = cli_framework::mcp::schema::command_to_tool_descriptor_full("myapp_tenant", &cmd);

    assert_eq!(presented.name, derived.name);
    assert_eq!(presented.description, "runtime description");
    assert_ne!(presented.description, derived.description);
    assert_eq!(presented.input_schema, presentation.input_schema);
    assert_ne!(
        presented.input_schema, derived.input_schema,
        "the derived schema must not be what is advertised"
    );
    assert_eq!(
        presented.meta, derived.meta,
        "_meta belongs to the Command, not to the presentation"
    );
    assert_eq!(
        presented.visibility, derived.visibility,
        "visibility belongs to the Command, not to the presentation"
    );
}

/// A presentation is advertising only: a tool carrying one dispatches to its
/// `command` exactly as the same tool without one. Covered here rather than
/// over HTTP because the stdio entry point (`dispatch_tool_call`, no identity)
/// is the shortest path that proves the presentation is never consulted.
#[tokio::test]
async fn a_presentation_does_not_reach_dispatch() {
    use cli_framework::mcp::{McpDynamicTool, McpToolPresentation};

    let provider: cli_framework::mcp::McpDynamicToolProvider = Arc::new(|_identity| {
        Box::pin(async move {
            vec![McpDynamicTool {
                name: "myapp_tenant".to_string(),
                command: tenant_tool("tenant-tool"),
                presentation: Some(McpToolPresentation {
                    description: "runtime description".to_string(),
                    // A schema that declares a required argument the caller
                    // below does not send: if the presentation were a gate,
                    // this call would fail.
                    input_schema: serde_json::json!({
                        "type": "object",
                        "properties": { "runtime_field": { "type": "string" } },
                        "required": ["runtime_field"],
                        "additionalProperties": false
                    }),
                }),
            }]
        })
    });
    let tool_registry = McpToolRegistry::from_command_registry(&CommandRegistry::new(), "myapp")
        .with_dynamic_tools(provider);

    let descriptors = tool_registry.list_tools_for_identity(None).await;
    assert_eq!(descriptors[0].description, "runtime description");

    let result = dispatch_tool_call(
        &tool_registry,
        "myapp_tenant",
        None,
        McpTransportKind::Stdio,
    )
    .await
    .expect("a presented tool must dispatch like any other");
    assert_eq!(call_result_text(&result).trim(), "tenant-tool");
}

/// Under stdio there is no HTTP request, so no identity can be established.
/// An installed provider is still consulted — with `None` — and what it
/// returns is dispatchable, which is why `dispatch_tool_call` (the
/// identity-less entry point) resolves a per-caller tool at all.
#[tokio::test]
async fn provider_is_invoked_with_none_identity_and_its_tools_dispatch() {
    let mut registry = CommandRegistry::new();
    registry.register(whoami_cmd());
    let tool_registry = McpToolRegistry::from_command_registry(&registry, "myapp")
        .with_dynamic_tools(always_provider());

    let names: Vec<String> = tool_registry
        .list_tools_for_identity(None)
        .await
        .iter()
        .map(|d| d.name.clone())
        .collect();
    assert!(
        names.contains(&"myapp_tenant".to_string()),
        "a None identity must still reach the provider: {names:?}"
    );

    let result = dispatch_tool_call(
        &tool_registry,
        "myapp_tenant",
        None,
        McpTransportKind::Stdio,
    )
    .await
    .expect("dispatch ok");
    assert_eq!(call_result_text(&result).trim(), "tenant-tool");
}

/// Static wins: a colliding name dispatches to the static command and is
/// listed exactly once.
#[tokio::test]
async fn static_command_wins_a_name_collision_with_the_provider() {
    let mut registry = CommandRegistry::new();
    registry.register(whoami_cmd());
    let tool_registry = McpToolRegistry::from_command_registry(&registry, "myapp")
        .with_dynamic_tools(always_provider());

    let result = dispatch_tool_call(
        &tool_registry,
        "myapp_whoami",
        None,
        McpTransportKind::Stdio,
    )
    .await
    .expect("dispatch ok");
    assert_eq!(
        call_result_text(&result).trim(),
        "anonymous",
        "the static whoami must answer, not the provider's stand-in"
    );

    let names: Vec<String> = tool_registry
        .list_tools_for_identity(None)
        .await
        .iter()
        .map(|d| d.name.clone())
        .collect();
    assert_eq!(
        names.iter().filter(|n| *n == "myapp_whoami").count(),
        1,
        "a colliding name must be listed once: {names:?}"
    );
}

/// A name no one serves still fails as `MCP_CMD_NOT_FOUND`, with a provider
/// installed — the fallback widens the tool set, it does not swallow misses.
#[tokio::test]
async fn unknown_tool_still_reports_not_found_with_a_provider_installed() {
    let tool_registry = McpToolRegistry::from_command_registry(&CommandRegistry::new(), "myapp")
        .with_dynamic_tools(always_provider());

    let err = dispatch_tool_call(&tool_registry, "myapp_nope", None, McpTransportKind::Stdio)
        .await
        .expect_err("unknown tool must fail");
    assert!(
        err.message.contains("MCP_CMD_NOT_FOUND"),
        "got: {}",
        err.message
    );
}

/// The stdio serve entry point stores an installed provider on the tool
/// registry, for parity with the HTTP entry point and the `AppBuilder`
/// wiring. Mirrors `serve_mcp_stdio_stores_installed_authenticator_without_blocking`:
/// this harness's stdin is already at EOF, so the call returns near-instantly
/// rather than blocking.
#[tokio::test]
async fn serve_mcp_stdio_stores_installed_dynamic_tools_without_blocking() {
    let registry = Arc::new(CommandRegistry::new());

    let result = tokio::time::timeout(
        std::time::Duration::from_secs(5),
        serve_mcp_stdio_opts_with_resources(
            registry,
            "testapp",
            cli_framework::security::CommandRiskPolicy::default(),
            cli_framework::mcp::McpToolExportPolicy::AllCommands,
            None,
            Arc::new(ResourceRegistry::new()),
            BannerSettings::from_env(),
            None,
            None,
            Some(always_provider()),
        ),
    )
    .await;

    assert!(
        result.is_ok(),
        "serve_mcp_stdio_opts_with_resources must not hang with a provider installed"
    );
}
