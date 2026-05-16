#[cfg(feature = "mcp-server")]
pub mod commands;
pub mod schema;
#[cfg(feature = "mcp-server")]
pub mod transport_http;
#[cfg(feature = "mcp-server")]
pub mod transport_stdio;

use crate::command::registry::CommandRegistry;
use crate::command::Command;
use crate::command::CommandArgs;
use crate::security::RiskEnforcer;
use crate::spec::value::ArgValue;
#[cfg(feature = "mcp-server")]
use anyhow::Result;
#[cfg(feature = "mcp-server")]
use rmcp::{
    model::{
        CallToolRequestParams, CallToolResult, Content, ErrorData, JsonObject, ListToolsResult,
        PaginatedRequestParams, ServerInfo, Tool,
    },
    service::RequestContext,
    RoleServer, ServerHandler,
};
use schema::{command_to_tool_descriptor, McpToolDescriptor};
use serde_json::Value;
#[cfg(feature = "mcp-server")]
use std::borrow::Cow;
use std::collections::HashMap;
#[cfg(feature = "mcp-server")]
use std::sync::Arc;
#[cfg(feature = "mcp-server")]
use tokio::sync::Mutex;

#[derive(Debug, Clone)]
pub struct McpServerArgs {
    pub host: String,
    pub port: u16,
    pub path: String,
}

impl Default for McpServerArgs {
    fn default() -> Self {
        Self {
            host: "127.0.0.1".to_string(),
            port: 8080,
            path: "/mcp".to_string(),
        }
    }
}

/// Controls which commands are registered as MCP tools.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum McpToolExportPolicy {
    /// Register all commands (backward-compatible default).
    /// `Command::expose_mcp` is ignored.
    #[default]
    AllCommands,
    /// Register only commands where `expose_mcp == true`.
    ExposeMcpOnly,
}

pub struct McpToolRegistry {
    tools: HashMap<String, Command>,
    app_name: String,
    risk_enforcer: RiskEnforcer,
    #[cfg(feature = "mcp-server")]
    gate: Option<std::sync::Arc<dyn McpToolGate>>,
}

impl McpToolRegistry {
    /// Backward-compatible constructor. Equivalent to calling
    /// `from_command_registry_with_policy(registry, app_name, McpToolExportPolicy::AllCommands)`.
    pub fn from_command_registry(registry: &CommandRegistry, app_name: &str) -> Self {
        Self::from_command_registry_with_policy(registry, app_name, McpToolExportPolicy::default())
    }

    /// Primary constructor. Applies `policy` to filter which commands become tools.
    pub fn from_command_registry_with_policy(
        registry: &CommandRegistry,
        app_name: &str,
        policy: McpToolExportPolicy,
    ) -> Self {
        if app_name == "unknown" {
            log::warn!("MCP: app_name is 'unknown'; use with_version() to set a proper name");
        }
        let mut tools = HashMap::new();
        for (path_str, cmd) in registry.all_tree_commands() {
            if policy == McpToolExportPolicy::ExposeMcpOnly && !cmd.expose_mcp {
                continue;
            }
            if cmd.spec.is_none() {
                log::warn!(
                    "MCP: command '{}' has no CommandSpec; using permissive schema",
                    cmd.id
                );
            }
            let tool_name = format!("{}.{}", app_name, path_str.replace('/', "."));
            tools.insert(tool_name, cmd.clone());
        }
        if tools.is_empty() && policy == McpToolExportPolicy::ExposeMcpOnly {
            log::warn!(
                "MCP: ExposeMcpOnly policy produced an empty tool set; \
                 no commands have expose_mcp: true"
            );
        }
        Self {
            tools,
            app_name: app_name.to_string(),
            risk_enforcer: RiskEnforcer::new(crate::security::CommandRiskPolicy::default()),
            #[cfg(feature = "mcp-server")]
            gate: None,
        }
    }

    pub fn with_risk_policy(mut self, policy: crate::security::CommandRiskPolicy) -> Self {
        self.risk_enforcer = RiskEnforcer::new(policy);
        self
    }

    #[cfg(feature = "mcp-server")]
    pub fn with_gate(mut self, gate: std::sync::Arc<dyn McpToolGate>) -> Self {
        self.gate = Some(gate);
        self
    }

    pub fn tool_count(&self) -> usize {
        self.tools.len()
    }

    pub fn list_tools(&self) -> Vec<McpToolDescriptor> {
        self.tools
            .iter()
            .map(|(name, cmd)| command_to_tool_descriptor(name, cmd.summary, cmd.spec.as_deref()))
            .collect()
    }

    pub fn resolve_tool(&self, tool_name: &str) -> Option<&Command> {
        self.tools.get(tool_name)
    }

    pub fn app_name(&self) -> &str {
        &self.app_name
    }
}

#[cfg(feature = "mcp-server")]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum McpTransportKind {
    Http,
    Stdio,
}

#[cfg(feature = "mcp-server")]
#[derive(Debug, Clone)]
pub struct McpToolCallContext {
    pub transport: McpTransportKind,
    pub tool_name: String,
    pub command_id: &'static str,
    pub command_category: Option<&'static str>,
    pub risk_tier: crate::security::CommandRiskTier,
}

#[cfg(feature = "mcp-server")]
#[async_trait::async_trait]
pub trait McpToolGate: Send + Sync {
    async fn before_execute(
        &self,
        ctx: &McpToolCallContext,
        args: &crate::command::CommandArgs,
    ) -> Result<(), McpToolGateError>;
}

#[cfg(feature = "mcp-server")]
#[derive(Debug, thiserror::Error)]
pub enum McpToolGateError {
    #[error("MCP_TOOL_DENIED: {message}")]
    Denied { message: String },

    #[error("MCP_TOOL_GATE_FAILED: {message}")]
    Failed { message: String },
}

#[cfg(feature = "mcp-server")]
struct McpAppContext;
#[cfg(feature = "mcp-server")]
impl crate::app::AppContext for McpAppContext {}

#[cfg(feature = "mcp-server")]
fn mcp_error(code: i32, message: String) -> ErrorData {
    ErrorData::new(rmcp::model::ErrorCode(code), Cow::Owned(message), None)
}

#[cfg(feature = "mcp-server")]
struct McpToolGateBridgeAdapter {
    gate: Option<Arc<dyn McpToolGate>>,
    transport: McpTransportKind,
    tool_name: String,
}

#[cfg(feature = "mcp-server")]
#[async_trait::async_trait]
impl crate::command_surface::tool_bridge::BridgeGate for McpToolGateBridgeAdapter {
    async fn before_execute(
        &self,
        cmd: &Command,
        args: &CommandArgs,
        tier: crate::security::command_risk::CommandRiskTier,
    ) -> Result<(), crate::command_surface::tool_bridge::BridgeError> {
        let Some(ref gate) = self.gate else {
            return Ok(());
        };
        let ctx = McpToolCallContext {
            transport: self.transport,
            tool_name: self.tool_name.clone(),
            command_id: cmd.id,
            command_category: cmd.category,
            risk_tier: tier,
        };

        match gate.before_execute(&ctx, args).await {
            Ok(()) => Ok(()),
            Err(e @ McpToolGateError::Denied { .. }) => {
                Err(crate::command_surface::tool_bridge::BridgeError::GateDenied(e.to_string()))
            }
            Err(e @ McpToolGateError::Failed { .. }) => {
                Err(crate::command_surface::tool_bridge::BridgeError::GateFailed(e.to_string()))
            }
        }
    }
}

pub(crate) fn json_value_to_arg_value(v: &Value) -> Option<ArgValue> {
    match v {
        Value::Bool(b) => Some(ArgValue::Bool(*b)),
        Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                Some(ArgValue::Int(i))
            } else {
                n.as_f64().map(ArgValue::Float)
            }
        }
        Value::String(s) => Some(ArgValue::Str(s.clone())),
        Value::Array(arr) => {
            let items: Vec<ArgValue> = arr.iter().filter_map(json_value_to_arg_value).collect();
            Some(ArgValue::List(items))
        }
        _ => None,
    }
}

/// Map JSON tool-call arguments into `CommandArgs` (stringly) and typed args (`ArgValue`).
///
/// Parity contract (used by MCP and `chat`):
/// - `_positional: [..]` maps to `CommandArgs.positional`
/// - all other keys map to `CommandArgs.named` via stringification
/// - typed values are converted via `json_value_to_arg_value`
pub(crate) fn map_mcp_args_to_command_args_from_json(
    arguments: Value,
) -> anyhow::Result<(CommandArgs, HashMap<String, ArgValue>)> {
    let obj = match arguments {
        Value::Null => serde_json::Map::new(),
        Value::Object(m) => m,
        other => {
            return Err(anyhow::anyhow!(
                "expected tool arguments to be an object, got {}",
                other
            ));
        }
    };

    let mut named = HashMap::new();
    let mut positional = Vec::new();
    let mut typed = HashMap::new();

    if let Some(Value::Array(pos)) = obj.get("_positional") {
        for v in pos {
            positional.push(match v {
                Value::String(s) => s.clone(),
                other => other.to_string(),
            });
        }
    }

    for (k, v) in &obj {
        if k == "_positional" {
            continue;
        }
        named.insert(
            k.clone(),
            match v {
                Value::String(s) => s.clone(),
                other => other.to_string(),
            },
        );
        if let Some(av) = json_value_to_arg_value(v) {
            typed.insert(k.clone(), av);
        }
    }

    Ok((CommandArgs { positional, named }, typed))
}

#[cfg(feature = "mcp-server")]
fn make_rmcp_tool(desc: &McpToolDescriptor) -> Tool {
    let input_schema: serde_json::Map<String, Value> = match &desc.input_schema {
        Value::Object(m) => m.clone(),
        _ => serde_json::Map::new(),
    };
    Tool::new(
        Cow::<'static, str>::Owned(desc.name.clone()),
        Cow::<'static, str>::Owned(desc.description.clone()),
        Arc::new(input_schema),
    )
}

#[cfg(feature = "mcp-server")]
#[derive(Clone)]
pub struct CliFrameworkHandler {
    tool_registry: Arc<McpToolRegistry>,
    transport: McpTransportKind,
    stdio_serialize: Option<Arc<Mutex<()>>>,
}

#[cfg(feature = "mcp-server")]
impl CliFrameworkHandler {
    pub fn new(tool_registry: Arc<McpToolRegistry>, transport: McpTransportKind) -> Self {
        Self {
            tool_registry,
            transport,
            stdio_serialize: None,
        }
    }

    pub fn with_stdio_serialization(mut self, lock: Arc<Mutex<()>>) -> Self {
        self.stdio_serialize = Some(lock);
        self
    }
}

#[cfg(feature = "mcp-server")]
impl ServerHandler for CliFrameworkHandler {
    fn get_info(&self) -> ServerInfo {
        ServerInfo::default()
    }

    fn list_tools(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> impl std::future::Future<Output = Result<ListToolsResult, ErrorData>> + Send + '_ {
        let descriptors = self.tool_registry.list_tools();
        let tools: Vec<Tool> = descriptors.iter().map(make_rmcp_tool).collect();
        std::future::ready(Ok(ListToolsResult {
            tools,
            next_cursor: None,
            meta: Default::default(),
        }))
    }

    fn call_tool(
        &self,
        request: CallToolRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> impl std::future::Future<Output = Result<CallToolResult, ErrorData>> + Send + '_ {
        let tool_name = request.name.to_string();
        let arguments = request.arguments;
        let registry = Arc::clone(&self.tool_registry);
        let transport = self.transport;
        let serialize = self.stdio_serialize.as_ref().map(Arc::clone);

        async move {
            if let Some(lock) = serialize {
                let _guard = lock.lock().await;
                dispatch_tool_call_spawned(registry, tool_name, arguments, transport).await
            } else {
                dispatch_tool_call_spawned(registry, tool_name, arguments, transport).await
            }
        }
    }
}

#[cfg(feature = "mcp-server")]
pub async fn dispatch_tool_call(
    tool_registry: &McpToolRegistry,
    tool_name: &str,
    arguments: Option<JsonObject>,
    transport: McpTransportKind,
) -> Result<CallToolResult, ErrorData> {
    use crate::command_surface::tool_bridge::{
        BridgeError, BridgeInput, BridgeInvocation, CommandAsToolBridge, ConfirmationMode,
    };

    let cmd = tool_registry.resolve_tool(tool_name).ok_or_else(|| {
        mcp_error(
            -32001,
            format!("MCP_CMD_NOT_FOUND: tool '{}' not registered", tool_name),
        )
    })?;

    let bridge = CommandAsToolBridge::new(tool_registry.risk_enforcer.policy().clone()).with_gate(
        Arc::new(McpToolGateBridgeAdapter {
            gate: tool_registry.gate.as_ref().map(Arc::clone),
            transport,
            tool_name: tool_name.to_string(),
        }),
    );

    let arguments_value = arguments.map(Value::Object).unwrap_or(Value::Null);
    let mut ctx = McpAppContext;

    match bridge
        .invoke(
            &mut ctx,
            BridgeInvocation {
                command: cmd,
                input: BridgeInput::Json(arguments_value),
                confirmation: ConfirmationMode::NonInteractive,
            },
        )
        .await
    {
        Ok(()) => Ok(CallToolResult::success(vec![Content::text("OK")])),
        Err(BridgeError::ArgValidation(msg)) => Err(mcp_error(
            -32002,
            format!("MCP_ARG_VALIDATION_FAILED: {}", msg),
        )),
        Err(BridgeError::GateDenied(msg)) => Err(mcp_error(-32005, msg)),
        Err(BridgeError::GateFailed(msg)) => Err(mcp_error(-32006, msg)),
        Err(BridgeError::Execution(e)) => {
            Err(mcp_error(-32003, format!("MCP_EXECUTION_FAILED: {}", e)))
        }
        Err(other) => Err(mcp_error(
            -32003,
            format!("MCP_EXECUTION_FAILED: {}", other),
        )),
    }
}

/// Dispatches a tool call in a separate tokio task (§4.7).
/// Panics in the task are caught as JoinError and returned as MCP_INTERNAL_ERROR.
#[cfg(feature = "mcp-server")]
pub async fn dispatch_tool_call_spawned(
    tool_registry: Arc<McpToolRegistry>,
    tool_name: String,
    arguments: Option<JsonObject>,
    transport: McpTransportKind,
) -> Result<CallToolResult, ErrorData> {
    let handle = tokio::spawn(async move {
        dispatch_tool_call(&tool_registry, &tool_name, arguments, transport).await
    });
    match handle.await {
        Ok(result) => result,
        Err(join_err) => Err(ErrorData::new(
            rmcp::model::ErrorCode(-32004),
            Cow::Owned(format!("MCP_INTERNAL_ERROR: task panicked: {}", join_err)),
            None,
        )),
    }
}

/// Convenience builder: constructs an `axum::Router` for MCP without binding a port.
///
/// Suitable for embedding MCP into an existing Axum application that already owns
/// a `TcpListener`. The caller MUST supply the same `app_name` they pass to
/// `AppBuilder::with_version` so tool names match the `{app_name}.{command}` convention.
///
/// # Example
///
/// ```rust,no_run
/// # use cli_framework::mcp::build_mcp_axum_router;
/// # use cli_framework::mcp::McpToolExportPolicy;
/// # use cli_framework::command::CommandRegistry;
/// # use cli_framework::security::CommandRiskPolicy;
/// let registry = CommandRegistry::new();
/// let router = build_mcp_axum_router(
///     &registry,
///     "myapp",
///     "/mcp",
///     CommandRiskPolicy::default(),
///     McpToolExportPolicy::default(),
/// );
/// // nest into your existing axum router:
/// // let app = axum::Router::new().merge(router);
/// ```
#[cfg(feature = "mcp-server")]
pub fn build_mcp_axum_router(
    registry: &CommandRegistry,
    app_name: &str,
    path: &str,
    risk_policy: crate::security::CommandRiskPolicy,
    export_policy: McpToolExportPolicy,
) -> axum::Router {
    let tool_registry = Arc::new(
        McpToolRegistry::from_command_registry_with_policy(registry, app_name, export_policy)
            .with_risk_policy(risk_policy),
    );
    transport_http::mcp_axum_router(tool_registry, path)
}

#[cfg(feature = "mcp-server")]
pub async fn serve_mcp(
    registry: Arc<CommandRegistry>,
    app_name: &str,
    args: McpServerArgs,
    risk_policy: crate::security::CommandRiskPolicy,
    export_policy: McpToolExportPolicy,
) -> Result<()> {
    serve_mcp_with_gate(registry, app_name, args, risk_policy, export_policy, None).await
}

#[cfg(feature = "mcp-server")]
pub async fn serve_mcp_with_gate(
    registry: Arc<CommandRegistry>,
    app_name: &str,
    args: McpServerArgs,
    risk_policy: crate::security::CommandRiskPolicy,
    export_policy: McpToolExportPolicy,
    gate: Option<std::sync::Arc<dyn McpToolGate>>,
) -> Result<()> {
    let mut tool_registry =
        McpToolRegistry::from_command_registry_with_policy(&registry, app_name, export_policy)
            .with_risk_policy(risk_policy);
    if let Some(gate) = gate {
        tool_registry = tool_registry.with_gate(gate);
    }
    let tool_registry = Arc::new(tool_registry);

    transport_http::start_streamable_http(tool_registry, &args).await
}

#[cfg(feature = "mcp-server")]
pub async fn serve_mcp_stdio(
    registry: Arc<CommandRegistry>,
    app_name: &str,
    risk_policy: crate::security::CommandRiskPolicy,
    export_policy: McpToolExportPolicy,
    gate: Option<std::sync::Arc<dyn McpToolGate>>,
) -> anyhow::Result<()> {
    let mut tool_registry =
        McpToolRegistry::from_command_registry_with_policy(&registry, app_name, export_policy)
            .with_risk_policy(risk_policy);
    if let Some(gate) = gate {
        tool_registry = tool_registry.with_gate(gate);
    }
    let tool_registry = Arc::new(tool_registry);
    transport_stdio::start_stdio(tool_registry).await
}
