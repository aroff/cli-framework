#[cfg(feature = "mcp-server")]
pub mod commands;
pub mod schema;
#[cfg(feature = "mcp-server")]
pub mod transport_http;
#[cfg(feature = "mcp-server")]
pub mod transport_stdio;

use crate::command::registry::CommandRegistry;
use crate::command::Command;
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
    gate: Option<std::sync::Arc<dyn crate::security::ExecutionGate>>,
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
            tracing::warn!("MCP: app_name is 'unknown'; use with_version() to set a proper name");
        }
        let mut tools = HashMap::new();
        for (path_str, cmd) in registry.all_tree_commands() {
            // Built-in `completion` is never exported as an MCP tool, regardless of policy.
            if path_str == "completion" {
                continue;
            }
            if policy == McpToolExportPolicy::ExposeMcpOnly && !cmd.expose_mcp {
                continue;
            }
            let tool_name = format!("{}_{}", app_name, path_str.replace('/', "_"));
            tools.insert(tool_name, cmd.clone());
        }
        if tools.is_empty() && policy == McpToolExportPolicy::ExposeMcpOnly {
            tracing::warn!(
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

    /// Build an `McpToolRegistry` directly from a pre-filtered command map.
    /// Keys MUST follow the `{app_name}_{path_underscored}` naming convention.
    /// No additional filtering is applied; caller is responsible for all exclusions.
    pub fn from_commands(commands: HashMap<String, Command>, app_name: &str) -> Self {
        Self {
            tools: commands,
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
    pub fn with_gate(mut self, gate: std::sync::Arc<dyn crate::security::ExecutionGate>) -> Self {
        self.gate = Some(gate);
        self
    }

    pub fn tool_count(&self) -> usize {
        self.tools.len()
    }

    pub fn list_tools(&self) -> Vec<McpToolDescriptor> {
        self.tools
            .iter()
            .map(|(name, cmd)| command_to_tool_descriptor(name, cmd.summary(), Some(&cmd.spec)))
            .collect()
    }

    pub fn resolve_tool(&self, tool_name: &str) -> Option<&Command> {
        self.tools.get(tool_name)
    }

    pub fn app_name(&self) -> &str {
        &self.app_name
    }

    pub fn risk_policy(&self) -> &crate::security::CommandRiskPolicy {
        self.risk_enforcer.policy()
    }
}

#[cfg(feature = "mcp-server")]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum McpTransportKind {
    Http,
    Stdio,
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
impl McpToolRegistry {
    fn bridge_for_call(
        &self,
        _transport: McpTransportKind,
        _tool_name: &str,
    ) -> crate::command_surface::tool_bridge::CommandAsToolBridge {
        use crate::command_surface::tool_bridge::CommandAsToolBridge;

        let bridge = CommandAsToolBridge::new(self.risk_enforcer.policy().clone());
        if let Some(gate) = self.gate.as_ref() {
            bridge.with_gate(Arc::clone(gate))
        } else {
            bridge
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

/// Map a JSON object of tool-call arguments into a typed `HashMap<String, ArgValue>`.
///
/// The `_positional` key is ignored (positional args are not supported in the typed map).
/// All other keys are converted via `json_value_to_arg_value`.
pub fn json_value_to_typed_map(
    json_obj: &serde_json::Map<String, Value>,
) -> HashMap<String, ArgValue> {
    json_obj
        .iter()
        .filter(|(k, _)| k.as_str() != "_positional")
        .filter_map(|(k, v)| json_value_to_arg_value(v).map(|av| (k.clone(), av)))
        .collect()
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
        BridgeError, BridgeInput, BridgeInvocation, ConfirmationMode,
    };

    let cmd = tool_registry.resolve_tool(tool_name).ok_or_else(|| {
        mcp_error(
            -32001,
            format!("MCP_CMD_NOT_FOUND: tool '{}' not registered", tool_name),
        )
    })?;

    let bridge = tool_registry.bridge_for_call(transport, tool_name);

    let arguments_value = arguments.map(Value::Object).unwrap_or(Value::Null);
    let mut ctx = McpAppContext;
    let res = bridge
        .invoke(
            &mut ctx,
            BridgeInvocation {
                command: cmd,
                input: BridgeInput::Json(arguments_value),
                confirmation: ConfirmationMode::NonInteractive,
                mode: crate::command_surface::tool_bridge::BridgeMode::Mcp,
            },
        )
        .await;

    match res {
        Ok(output) => Ok(CallToolResult::success(vec![Content::text(
            if output.is_empty() { "OK" } else { &output },
        )])),
        Err(BridgeError::ArgValidation(msg)) => Err(mcp_error(
            -32002,
            format!("MCP_ARG_VALIDATION_FAILED: {}", msg),
        )),
        Err(BridgeError::GateDenied(msg)) => {
            Err(mcp_error(-32005, format!("MCP_TOOL_DENIED: {}", msg)))
        }
        Err(BridgeError::GateFailed(msg)) => {
            Err(mcp_error(-32006, format!("MCP_TOOL_GATE_FAILED: {}", msg)))
        }
        Err(BridgeError::Execution(e)) => {
            Err(mcp_error(-32003, format!("MCP_EXECUTION_FAILED: {}", e)))
        }
        Err(BridgeError::ToolNotFound(_)) => Err(mcp_error(
            -32001,
            format!("MCP_CMD_NOT_FOUND: tool '{}' not registered", tool_name),
        )),
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
/// `AppBuilder::with_version` so tool names match the `{app_name}_{command}` convention.
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
pub async fn serve_mcp_with_gate(
    registry: Arc<CommandRegistry>,
    app_name: &str,
    args: McpServerArgs,
    risk_policy: crate::security::CommandRiskPolicy,
    export_policy: McpToolExportPolicy,
    gate: Option<std::sync::Arc<dyn crate::security::ExecutionGate>>,
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
    gate: Option<std::sync::Arc<dyn crate::security::ExecutionGate>>,
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
