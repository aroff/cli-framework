pub mod banner;
#[cfg(feature = "mcp-server")]
pub mod commands;
#[cfg(feature = "mcp-server")]
pub mod resources;
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
pub use banner::BannerSettings;
#[cfg(feature = "mcp-server")]
use rmcp::{
    model::{
        CallToolRequestParams, CallToolResult, Content, ErrorData, JsonObject, ListResourcesResult,
        ListToolsResult, Meta, PaginatedRequestParams, RawResource, ReadResourceRequestParams,
        ReadResourceResult, Resource, ResourceContents, ServerCapabilities, ServerInfo, Tool,
    },
    service::RequestContext,
    RoleServer, ServerHandler,
};
use schema::{command_to_tool_descriptor_full, McpToolDescriptor};
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
            .map(|(name, cmd)| command_to_tool_descriptor_full(name, cmd))
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

/// `AppContext` used for MCP tool dispatch.
///
/// Captures both the text a command emits via `framework_println` (returned as
/// the tool result `content`) and any structured value it attaches via
/// `framework_set_structured_content` (returned as the result
/// `structuredContent`, CF-7). Without this capture an MCP tool would print to
/// the server's stdout and report only `"OK"`.
#[cfg(feature = "mcp-server")]
struct McpAppContext {
    buffer: std::sync::Mutex<Vec<u8>>,
    structured: std::sync::Mutex<Option<Value>>,
}
#[cfg(feature = "mcp-server")]
impl McpAppContext {
    fn new() -> Self {
        Self {
            buffer: std::sync::Mutex::new(Vec::new()),
            structured: std::sync::Mutex::new(None),
        }
    }
}
#[cfg(feature = "mcp-server")]
impl crate::app::AppContext for McpAppContext {
    fn framework_println(&self, s: &str) {
        use std::io::Write;
        let mut buf = self.buffer.lock().unwrap();
        let _ = writeln!(buf, "{}", s);
    }

    fn drain_output(&self) -> String {
        let mut buf = self.buffer.lock().unwrap();
        let data = std::mem::take(&mut *buf);
        String::from_utf8_lossy(&data).into_owned()
    }

    fn framework_set_structured_content(&self, value: Value) {
        *self.structured.lock().unwrap() = Some(value);
    }

    fn drain_structured_content(&self) -> Option<Value> {
        self.structured.lock().unwrap().take()
    }
}

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
    let mut tool = Tool::new(
        Cow::<'static, str>::Owned(desc.name.clone()),
        Cow::<'static, str>::Owned(desc.description.clone()),
        Arc::new(input_schema),
    );

    // rmcp 1.6 `Tool` carries a per-tool `_meta` passthrough (`Tool::meta`,
    // serialized as `_meta`) but has NO `visibility` field. We therefore merge
    // the command's opaque `_meta` value AND the `visibility` tags into a single
    // `_meta` object so both survive on the wire (see R1). The opaque `_meta`
    // contents are owned by the consumer; `visibility` rides in `_meta.visibility`.
    let mut meta = Meta::new();
    if let Some(Value::Object(m)) = &desc.meta {
        for (k, v) in m {
            meta.insert(k.clone(), v.clone());
        }
    }
    if let Some(visibility) = &desc.visibility {
        meta.insert(
            "visibility".to_string(),
            Value::Array(
                visibility
                    .iter()
                    .map(|s| Value::String(s.clone()))
                    .collect(),
            ),
        );
    }
    if !meta.is_empty() {
        tool.meta = Some(meta);
    }

    tool
}

/// Convert a [`resources::UiResource`] into an rmcp `ResourceContents`,
/// emitting any opaque per-resource `_meta` at `contents[]._meta`.
///
/// The `_meta` value is passed through verbatim; cli-framework does not
/// interpret it (the consumer owns its shape).
#[cfg(feature = "mcp-server")]
fn ui_resource_to_contents(uri: &str, resource: resources::UiResource) -> ResourceContents {
    use resources::UiResourceBody;

    let mut base = match resource.body {
        UiResourceBody::Text(text) => ResourceContents::TextResourceContents {
            uri: uri.to_string(),
            mime_type: Some(resource.mime_type),
            text,
            meta: None,
        },
        UiResourceBody::Blob(blob) => ResourceContents::BlobResourceContents {
            uri: uri.to_string(),
            mime_type: Some(resource.mime_type),
            blob,
            meta: None,
        },
    };

    if let Some(Value::Object(m)) = resource.meta {
        let mut meta = Meta::new();
        for (k, v) in m {
            meta.insert(k, v);
        }
        if !meta.is_empty() {
            base = base.with_meta(meta);
        }
    }

    base
}

#[cfg(feature = "mcp-server")]
#[derive(Clone)]
pub struct CliFrameworkHandler {
    tool_registry: Arc<McpToolRegistry>,
    resource_registry: Arc<resources::ResourceRegistry>,
    transport: McpTransportKind,
    stdio_serialize: Option<Arc<Mutex<()>>>,
}

#[cfg(feature = "mcp-server")]
impl CliFrameworkHandler {
    pub fn new(tool_registry: Arc<McpToolRegistry>, transport: McpTransportKind) -> Self {
        Self {
            tool_registry,
            resource_registry: Arc::new(resources::ResourceRegistry::new()),
            transport,
            stdio_serialize: None,
        }
    }

    /// Attach a resource registry so this handler serves `resources/list` and
    /// `resources/read` for the registered resource URIs.
    pub fn with_resource_registry(
        mut self,
        resource_registry: Arc<resources::ResourceRegistry>,
    ) -> Self {
        self.resource_registry = resource_registry;
        self
    }

    pub fn with_stdio_serialization(mut self, lock: Arc<Mutex<()>>) -> Self {
        self.stdio_serialize = Some(lock);
        self
    }

    /// Build the `resources/list` result from the resource registry.
    ///
    /// Transport-independent seam used by the [`ServerHandler::list_resources`]
    /// impl and by in-process tests.
    pub fn list_resources_result(&self) -> ListResourcesResult {
        let resources: Vec<Resource> = self
            .resource_registry
            .listings()
            .into_iter()
            .map(|listing| {
                let mut raw = RawResource::new(listing.uri, listing.name);
                raw.description = listing.description;
                raw.mime_type = listing.mime_type;
                Resource::new(raw, None)
            })
            .collect();
        ListResourcesResult {
            resources,
            next_cursor: None,
            meta: Default::default(),
        }
    }

    /// Read a single resource by URI, building the `resources/read` result.
    ///
    /// Transport-independent seam used by the [`ServerHandler::read_resource`]
    /// impl and by in-process tests. Returns `MCP_RESOURCE_NOT_FOUND`
    /// when the URI is not registered (or its provider yields nothing).
    pub fn read_resource_uri(&self, uri: &str) -> Result<ReadResourceResult, ErrorData> {
        match self.resource_registry.read(uri) {
            Some(resource) => {
                let contents = ui_resource_to_contents(uri, resource);
                Ok(ReadResourceResult::new(vec![contents]))
            }
            None => Err(mcp_error(
                -32002,
                format!("MCP_RESOURCE_NOT_FOUND: resource '{}' not registered", uri),
            )),
        }
    }
}

#[cfg(feature = "mcp-server")]
impl ServerHandler for CliFrameworkHandler {
    fn get_info(&self) -> ServerInfo {
        // Advertise tools always; advertise resources only when some are
        // registered, so hosts without a resource registry see a tools-only
        // server (backward compatible). The capabilities builder is type-state
        // encoded, so the two cases are built on separate paths.
        let capabilities = if self.resource_registry.is_empty() {
            ServerCapabilities::builder().enable_tools().build()
        } else {
            ServerCapabilities::builder()
                .enable_tools()
                .enable_resources()
                .build()
        };
        let mut info = ServerInfo::default();
        info.capabilities = capabilities;
        info
    }

    fn list_resources(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> impl std::future::Future<Output = Result<ListResourcesResult, ErrorData>> + Send + '_ {
        std::future::ready(Ok(self.list_resources_result()))
    }

    fn read_resource(
        &self,
        request: ReadResourceRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> impl std::future::Future<Output = Result<ReadResourceResult, ErrorData>> + Send + '_ {
        std::future::ready(self.read_resource_uri(&request.uri))
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
    let mut ctx = McpAppContext::new();
    let res = bridge
        .invoke_structured(
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
        Ok(output) => {
            let text = if output.text.is_empty() {
                "OK"
            } else {
                &output.text
            };
            // CF-7: a command may attach a `structuredContent` value distinct
            // from the `content` text (e.g. server-rendered View HTML), kept out
            // of the model's text context.
            let mut result = CallToolResult::success(vec![Content::text(text)]);
            result.structured_content = output.structured;
            Ok(result)
        }
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
    build_mcp_axum_router_with_resources(
        registry,
        app_name,
        path,
        risk_policy,
        export_policy,
        Arc::new(resources::ResourceRegistry::new()),
    )
}

/// Like [`build_mcp_axum_router`], but threads a populated
/// [`resources::ResourceRegistry`] into the served handler so registered
/// `ui://…` resources are served via `resources/list` and `resources/read`.
///
/// This is the HTTP-side seam for consumers (e.g. an MCP-Apps binding) that
/// mount MCP into an existing Axum app via [`crate::api::ApiServer::mcp_router`].
///
/// # Example
///
/// ```rust,no_run
/// # use cli_framework::mcp::build_mcp_axum_router_with_resources;
/// # use cli_framework::mcp::McpToolExportPolicy;
/// # use cli_framework::mcp::resources::{ResourceRegistry, UiResource};
/// # use cli_framework::command::CommandRegistry;
/// # use cli_framework::security::CommandRiskPolicy;
/// # use std::sync::Arc;
/// let registry = CommandRegistry::new();
/// let mut resources = ResourceRegistry::new();
/// resources.register_static(
///     "ui://app/index.html",
///     "App shell",
///     UiResource::html("<!doctype html><title>App</title>"),
/// );
/// let router = build_mcp_axum_router_with_resources(
///     &registry,
///     "myapp",
///     "/mcp",
///     CommandRiskPolicy::default(),
///     McpToolExportPolicy::default(),
///     Arc::new(resources),
/// );
/// ```
#[cfg(feature = "mcp-server")]
pub fn build_mcp_axum_router_with_resources(
    registry: &CommandRegistry,
    app_name: &str,
    path: &str,
    risk_policy: crate::security::CommandRiskPolicy,
    export_policy: McpToolExportPolicy,
    resource_registry: Arc<resources::ResourceRegistry>,
) -> axum::Router {
    let tool_registry = Arc::new(
        McpToolRegistry::from_command_registry_with_policy(registry, app_name, export_policy)
            .with_risk_policy(risk_policy),
    );
    transport_http::mcp_axum_router_with_resources(tool_registry, resource_registry, path)
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
    serve_mcp_with_gate_opts(
        registry,
        app_name,
        args,
        risk_policy,
        export_policy,
        gate,
        BannerSettings::from_env(),
    )
    .await
}

/// Like [`serve_mcp_with_gate`], but with explicit startup-banner settings
/// (resolved from `--quiet` / `--json` conventions by the `mcp serve` command).
#[cfg(feature = "mcp-server")]
#[allow(clippy::too_many_arguments)]
pub async fn serve_mcp_with_gate_opts(
    registry: Arc<CommandRegistry>,
    app_name: &str,
    args: McpServerArgs,
    risk_policy: crate::security::CommandRiskPolicy,
    export_policy: McpToolExportPolicy,
    gate: Option<std::sync::Arc<dyn crate::security::ExecutionGate>>,
    banner: BannerSettings,
) -> Result<()> {
    serve_mcp_with_gate_opts_with_resources(
        registry,
        app_name,
        args,
        risk_policy,
        export_policy,
        gate,
        Arc::new(resources::ResourceRegistry::new()),
        banner,
    )
    .await
}

/// Like [`serve_mcp_with_gate_opts`], but threads a populated
/// [`resources::ResourceRegistry`] into the served handler so registered
/// `ui://…` resources are served over the Streamable HTTP transport.
#[cfg(feature = "mcp-server")]
#[allow(clippy::too_many_arguments)]
pub async fn serve_mcp_with_gate_opts_with_resources(
    registry: Arc<CommandRegistry>,
    app_name: &str,
    args: McpServerArgs,
    risk_policy: crate::security::CommandRiskPolicy,
    export_policy: McpToolExportPolicy,
    gate: Option<std::sync::Arc<dyn crate::security::ExecutionGate>>,
    resource_registry: Arc<resources::ResourceRegistry>,
    banner: BannerSettings,
) -> Result<()> {
    let mut tool_registry =
        McpToolRegistry::from_command_registry_with_policy(&registry, app_name, export_policy)
            .with_risk_policy(risk_policy);
    if let Some(gate) = gate {
        tool_registry = tool_registry.with_gate(gate);
    }
    let tool_registry = Arc::new(tool_registry);

    transport_http::start_streamable_http_with_resources(
        tool_registry,
        resource_registry,
        &args,
        banner,
    )
    .await
}

#[cfg(feature = "mcp-server")]
pub async fn serve_mcp_stdio(
    registry: Arc<CommandRegistry>,
    app_name: &str,
    risk_policy: crate::security::CommandRiskPolicy,
    export_policy: McpToolExportPolicy,
    gate: Option<std::sync::Arc<dyn crate::security::ExecutionGate>>,
) -> anyhow::Result<()> {
    serve_mcp_stdio_opts(
        registry,
        app_name,
        risk_policy,
        export_policy,
        gate,
        BannerSettings::from_env(),
    )
    .await
}

/// Like [`serve_mcp_stdio`], but with explicit startup-banner settings.
#[cfg(feature = "mcp-server")]
pub async fn serve_mcp_stdio_opts(
    registry: Arc<CommandRegistry>,
    app_name: &str,
    risk_policy: crate::security::CommandRiskPolicy,
    export_policy: McpToolExportPolicy,
    gate: Option<std::sync::Arc<dyn crate::security::ExecutionGate>>,
    banner: BannerSettings,
) -> anyhow::Result<()> {
    serve_mcp_stdio_opts_with_resources(
        registry,
        app_name,
        risk_policy,
        export_policy,
        gate,
        Arc::new(resources::ResourceRegistry::new()),
        banner,
    )
    .await
}

/// Like [`serve_mcp_stdio_opts`], but threads a populated
/// [`resources::ResourceRegistry`] into the served handler so registered
/// `ui://…` resources are served over the stdio transport.
#[cfg(feature = "mcp-server")]
#[allow(clippy::too_many_arguments)]
pub async fn serve_mcp_stdio_opts_with_resources(
    registry: Arc<CommandRegistry>,
    app_name: &str,
    risk_policy: crate::security::CommandRiskPolicy,
    export_policy: McpToolExportPolicy,
    gate: Option<std::sync::Arc<dyn crate::security::ExecutionGate>>,
    resource_registry: Arc<resources::ResourceRegistry>,
    banner: BannerSettings,
) -> anyhow::Result<()> {
    let mut tool_registry =
        McpToolRegistry::from_command_registry_with_policy(&registry, app_name, export_policy)
            .with_risk_policy(risk_policy);
    if let Some(gate) = gate {
        tool_registry = tool_registry.with_gate(gate);
    }
    let tool_registry = Arc::new(tool_registry);
    transport_stdio::start_stdio_with_resources(tool_registry, resource_registry, banner).await
}

#[cfg(all(test, feature = "mcp-server"))]
mod rmcp_tool_meta_tests {
    use super::*;
    use crate::mcp::schema::McpToolDescriptor;

    // R1: confirm the live `rmcp::model::Tool` carries the opaque `_meta`
    // passthrough AND the `visibility` tags (which have no native `Tool` field)
    // when serialized to the wire. The `_meta` value is consumer-owned and
    // passed through verbatim — here a neutral opaque object.
    #[test]
    fn make_rmcp_tool_serializes_opaque_meta_and_visibility() {
        let desc = McpToolDescriptor {
            name: "es_detail".to_string(),
            description: "Open detail".to_string(),
            input_schema: serde_json::json!({ "type": "object" }),
            meta: Some(serde_json::json!({
                "x_consumer": { "key": "value" }
            })),
            visibility: Some(vec!["app".to_string()]),
        };
        let tool = make_rmcp_tool(&desc);
        let json = serde_json::to_value(&tool).unwrap();
        assert_eq!(json["_meta"]["x_consumer"]["key"], "value");
        assert_eq!(json["_meta"]["visibility"], serde_json::json!(["app"]));
    }

    #[test]
    fn make_rmcp_tool_without_meta_omits_meta_key() {
        let desc = McpToolDescriptor {
            name: "es_plain".to_string(),
            description: "Plain".to_string(),
            input_schema: serde_json::json!({ "type": "object" }),
            meta: None,
            visibility: None,
        };
        let tool = make_rmcp_tool(&desc);
        let json = serde_json::to_value(&tool).unwrap();
        assert!(json.get("_meta").is_none(), "got: {json}");
    }
}

#[cfg(all(test, feature = "mcp-server"))]
mod cf7_structured_content_tests {
    use super::*;
    use crate::command::Command;
    use crate::spec::command_tree::CommandSpec;
    use std::collections::HashMap;

    // CF-7: a command's execute can attach `structuredContent` distinct from the
    // model-facing `content` text, and the MCP dispatch surfaces both.
    #[tokio::test]
    async fn dispatch_carries_structured_content_distinct_from_text() {
        let cmd = Command {
            id: Arc::from("view"),
            spec: Arc::new(CommandSpec {
                summary: "render a view",
                ..Default::default()
            }),
            validator: None,
            expose_mcp: true,
            expose_chat: false,
            meta: None,
            visibility: None,
            execute: Arc::new(|ctx, _args| {
                Box::pin(async move {
                    ctx.framework_println("text fallback for the model");
                    ctx.framework_set_structured_content(
                        serde_json::json!({ "html": "<article>hi</article>" }),
                    );
                    Ok(())
                })
            }),
        };

        let mut commands = HashMap::new();
        commands.insert("app_view".to_string(), cmd);
        let registry = McpToolRegistry::from_commands(commands, "app");

        let result = dispatch_tool_call(&registry, "app_view", None, McpTransportKind::Stdio)
            .await
            .expect("dispatch ok");

        // structuredContent carries the HTML; the model-facing text does not.
        let structured = result.structured_content.expect("structured content set");
        assert_eq!(structured["html"], "<article>hi</article>");
        let text = match &result.content[0].raw {
            rmcp::model::RawContent::Text(t) => t.text.clone(),
            other => panic!("expected text content, got {other:?}"),
        };
        assert_eq!(text, "text fallback for the model\n");
        assert!(
            !text.contains("<article>"),
            "HTML must not leak into content"
        );
    }

    // A command that sets no structured content yields a None structured field
    // (backward compatible).
    #[tokio::test]
    async fn dispatch_without_structured_content_is_none() {
        let cmd = Command {
            id: Arc::from("plain"),
            spec: Arc::new(CommandSpec {
                summary: "plain",
                ..Default::default()
            }),
            validator: None,
            expose_mcp: true,
            expose_chat: false,
            meta: None,
            visibility: None,
            execute: Arc::new(|ctx, _args| {
                Box::pin(async move {
                    ctx.framework_println("ok");
                    Ok(())
                })
            }),
        };
        let mut commands = HashMap::new();
        commands.insert("app_plain".to_string(), cmd);
        let registry = McpToolRegistry::from_commands(commands, "app");

        let result = dispatch_tool_call(&registry, "app_plain", None, McpTransportKind::Stdio)
            .await
            .expect("dispatch ok");
        assert!(result.structured_content.is_none());
    }
}
