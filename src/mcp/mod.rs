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
        CallToolRequestParams, CallToolResult, ContentBlock, ErrorData, JsonObject,
        ListResourcesResult, ListToolsResult, Meta, PaginatedRequestParams,
        ReadResourceRequestParams, ReadResourceResult, Resource, ResourceContents,
        ServerCapabilities, ServerInfo, Tool,
    },
    service::RequestContext,
    RoleServer, ServerHandler,
};
use schema::{command_to_tool_descriptor_full, McpToolDescriptor};
use serde_json::Value;
#[cfg(feature = "mcp-server")]
use std::any::Any;
#[cfg(feature = "mcp-server")]
use std::borrow::Cow;
use std::collections::HashMap;
#[cfg(feature = "mcp-server")]
use std::sync::Arc;
#[cfg(feature = "mcp-server")]
use tokio::sync::Mutex;

/// A per-request identity hook for the MCP HTTP transport.
///
/// cli-framework stays unopinionated about authentication: it never parses
/// bearer tokens, validates JWTs, or knows about JWKS/OIDC. Instead, the host
/// (e.g. `AppBuilder::with_mcp_request_authenticator`) supplies a closure that
/// maps the incoming HTTP request's headers to an opaque, type-erased
/// identity value — for example, a downstream product's own `SecurityContext`
/// built from a validated Bearer token.
///
/// The MCP HTTP transport calls this closure once per HTTP request (never
/// under stdio, where no HTTP request exists) and stashes the returned value
/// so the tool's `execute` closure can read it back via
/// [`crate::app::RequestIdentityExt::request_identity`]. Returning `None`
/// (missing/invalid header, anonymous caller, etc.) is a normal outcome, not
/// an error — cli-framework does not reject or short-circuit the call based
/// on this hook; enforcement is entirely up to the host's own tool logic.
#[cfg(feature = "mcp-server")]
pub type McpRequestAuthenticator =
    Arc<dyn Fn(&http::HeaderMap) -> Option<Arc<dyn Any + Send + Sync>> + Send + Sync>;

/// The future returned by an [`McpDynamicToolProvider`].
///
/// Owned and `'static`: the provider is handed an owned identity, so the
/// future borrows nothing from the registry and can be awaited freely inside
/// a spawned dispatch task.
#[cfg(feature = "mcp-server")]
pub type McpDynamicToolsFuture =
    std::pin::Pin<Box<dyn std::future::Future<Output = Vec<(String, Command)>> + Send + 'static>>;

/// A per-caller tool-set hook for the MCP surface.
///
/// Companion to [`McpRequestAuthenticator`]: the authenticator turns request
/// headers into an opaque identity, and this provider turns that identity into
/// the extra commands *that caller* may see and invoke — for example the
/// actions of the tenant-scoped plugins installed for the authenticated user.
/// Returned pairs are `(tool_name, command)`; the name MUST already follow the
/// `{app_name}_{path_underscored}` convention, exactly as for
/// [`McpToolRegistry::from_commands`] — cli-framework does not rewrite it.
///
/// A `Vec` rather than a `HashMap` deliberately: it lets the provider fix the
/// order in which its tools appear in `tools/list` (a map would reintroduce
/// the very nondeterminism this surface is trying to bound), and it costs
/// nothing at the sizes involved.
///
/// The callback is async because real providers read a database to decide
/// visibility.
#[cfg(feature = "mcp-server")]
pub type McpDynamicToolProvider =
    Arc<dyn Fn(Option<Arc<dyn Any + Send + Sync>>) -> McpDynamicToolsFuture + Send + Sync>;

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
    telemetry: Option<std::sync::Arc<dyn crate::telemetry::Telemetry + Send + Sync>>,
    #[cfg(feature = "mcp-server")]
    request_authenticator: Option<McpRequestAuthenticator>,
    #[cfg(feature = "mcp-server")]
    dynamic_tools: Option<McpDynamicToolProvider>,
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
            // A `completion` command is never an MCP tool, regardless of which
            // namespace contains it. Authentication commands remain excluded
            // by their established command family.
            if registry.is_framework_completion(path_str)
                || path_str.starts_with("auth/")
                || path_str == "auth"
            {
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
            telemetry: None,
            #[cfg(feature = "mcp-server")]
            request_authenticator: None,
            #[cfg(feature = "mcp-server")]
            dynamic_tools: None,
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
            telemetry: None,
            #[cfg(feature = "mcp-server")]
            request_authenticator: None,
            #[cfg(feature = "mcp-server")]
            dynamic_tools: None,
        }
    }

    pub fn with_risk_policy(mut self, policy: crate::security::CommandRiskPolicy) -> Self {
        self.risk_enforcer = RiskEnforcer::new(policy);
        self
    }

    /// Attach a telemetry handle so MCP tool calls emit `cli.command` spans.
    pub fn with_telemetry(
        mut self,
        handle: std::sync::Arc<dyn crate::telemetry::Telemetry + Send + Sync>,
    ) -> Self {
        self.telemetry = Some(handle);
        self
    }

    #[cfg(feature = "mcp-server")]
    pub fn with_gate(mut self, gate: std::sync::Arc<dyn crate::security::ExecutionGate>) -> Self {
        self.gate = Some(gate);
        self
    }

    /// Install a per-request identity hook (see [`McpRequestAuthenticator`]).
    ///
    /// Opt-in: when unset, every MCP tool call sees `request_identity() ==
    /// None` — identical to behavior before this hook existed.
    #[cfg(feature = "mcp-server")]
    pub fn with_request_authenticator(mut self, authenticator: McpRequestAuthenticator) -> Self {
        self.request_authenticator = Some(authenticator);
        self
    }

    /// Install a per-caller tool-set hook (see [`McpDynamicToolProvider`]).
    ///
    /// The provider is handed the opaque per-request identity produced by an
    /// installed [`McpRequestAuthenticator`] and returns the *additional*
    /// commands that caller may see and invoke, on top of the static tool set
    /// fixed at construction. Both `tools/list` and `tools/call` consume it, so
    /// what a caller is shown is by construction what that caller can dispatch.
    ///
    /// # This is discovery, not a security boundary
    ///
    /// Omitting a tool from a caller's `tools/list` hides it; it does **not**
    /// authorize anything, and returning a tool does **not** authorize the
    /// caller to run it. Consumers MUST still enforce authorization inside the
    /// command's own `execute` closure, reading the identity back via
    /// [`crate::app::RequestIdentityExt::request_identity`]. Reasons, all of
    /// which hold even with a correct provider:
    ///
    /// - MCP clients call `tools/call` with any name they like. They are not
    ///   obliged to have called `tools/list` first, nor to call only what it
    ///   returned.
    /// - Every command in the **static** set stays callable by every caller,
    ///   whatever the provider returns — the provider only ever *adds*.
    /// - Tool names are guessable. A caller who learns another tenant's tool
    ///   name can ask for it; whether that call succeeds is decided by the
    ///   command body, never by this hook.
    /// - Under stdio, and on an unauthenticated HTTP request, the provider is
    ///   invoked with `None` (see below) and cannot distinguish callers at all.
    ///
    /// Two adjacent boundaries, stated so they are not guessed at:
    ///
    /// - [`McpToolExportPolicy`] and `Command::expose_mcp` filter the **static**
    ///   set when the registry is built; they do **not** filter this hook. A
    ///   command the provider returns is exported even with
    ///   `expose_mcp: false` under
    ///   [`McpToolExportPolicy::ExposeMcpOnly`] — the provider is the only
    ///   filter for its own commands.
    /// - What *does* still apply to a per-caller tool, exactly as to a static
    ///   one: argument validation, the command risk policy, and any
    ///   [`with_gate`](Self::with_gate) execution gate, because dispatch builds
    ///   the same bridge whatever the command's origin. Read the risk-policy
    ///   half of that carefully:
    ///   [`CommandRiskPolicy::classify`](crate::security::CommandRiskPolicy::classify)
    ///   keys on `Command.id` and then on the command's category, and a
    ///   provider-built command's id is by definition absent from the
    ///   consumer's `tiers` map. Unless the provider sets a category
    ///   (`admin`/`deployment`/`destructive` → `Destructive`, `data`/`config`
    ///   → `Sensitive`), the tool lands on `default_tier` — `Safe` by default.
    ///   "The risk policy applies" does **not** mean a consumer's destructive
    ///   tier covers per-caller tools; the provider must classify what it
    ///   returns. Note the gate's own limit, too: [`crate::security::ExecutionGate::before_execute`]
    ///   receives the command, its arguments and its risk tier — **not** the
    ///   caller identity. A gate can therefore stop a whole class of calls, but
    ///   it cannot answer "may *this* caller do this". Per-caller authorization
    ///   belongs in the command's own `execute`, which can read the identity
    ///   back via [`crate::app::RequestIdentityExt::request_identity`].
    ///
    /// # Precedence and determinism
    ///
    /// - **Static wins.** A provided command whose name collides with a static
    ///   tool is ignored: `tools/call` resolves the name to the static command,
    ///   and `tools/list` emits the static entry only — never a duplicate. The
    ///   same applies within one provider result: the first pair for a given
    ///   name wins. Neither drop is silent — each is reported at `tracing::warn!`,
    ///   naming the tool and which rule dropped it, because a provider that
    ///   believes it published a tool has no other way to notice it never
    ///   reached the client. Nothing is rejected: the rest of the result is
    ///   listed as normal.
    /// - **Never cached.** The set is recomputed for every request, from that
    ///   request's identity. cli-framework keeps no per-identity cache, so
    ///   `tools/list` and `tools/call` cannot disagree about what a caller has.
    ///   A revocation therefore takes effect on the caller's next request *to
    ///   this server*, which is not the same as the caller seeing it: the
    ///   `tools/call` path re-runs the provider and so is prompt, while
    ///   `tools/list` is only as fresh as the client's last call to it. This
    ///   server advertises `tools` **without** `listChanged` and never emits
    ///   `notifications/tools/list_changed`, so a client that listed once at
    ///   session start keeps showing a withdrawn tool until it re-lists.
    ///   Revocation is enforced at dispatch, not at discovery — another reason
    ///   discovery is not the boundary.
    /// - **The miss path is caller-driven work.** One provider invocation per
    ///   `tools/list`, and one per `tools/call` whose name is *not* in the
    ///   static set. That second rate is chosen by the caller, including an
    ///   unauthenticated one: a loop of `tools/call` with random names is a
    ///   provider invocation — typically a database read — per request, with
    ///   no cache in front of it. Treat it as a rate-limiting question at the
    ///   edge, not as a performance footnote. Caching inside the provider is
    ///   the consumer's call, along with the staleness it introduces.
    /// - `tools/call` invokes the provider **only** when the requested name is
    ///   absent from the static set, so the common path costs nothing.
    ///
    /// # Identity may be absent
    ///
    /// The provider is called with `None` whenever no identity was established
    /// for the request: under stdio (there is no HTTP request), when no
    /// [`McpRequestAuthenticator`] is installed, and when the installed
    /// authenticator returned `None` for these headers (missing or malformed
    /// credentials). cli-framework does not treat that as an error and does not
    /// skip the provider — the consumer decides what an anonymous caller sees,
    /// which may legitimately be an empty `Vec`.
    ///
    /// Opt-in: when unset, both `tools/list` and `tools/call` behave exactly as
    /// they did before this hook existed, on every transport. In particular
    /// `tools/list` does not run an installed [`McpRequestAuthenticator`] at
    /// all when no provider is installed: it could not use the result, and the
    /// authenticator is consumer code that may log, emit metrics or spend a
    /// rate-limit budget. Installing a provider is therefore what starts
    /// authenticating `tools/list` requests.
    ///
    /// # Example
    ///
    /// ```rust,no_run
    /// # use cli_framework::mcp::{McpToolRegistry, McpDynamicToolProvider};
    /// # use cli_framework::command::CommandRegistry;
    /// # use std::sync::Arc;
    /// # struct CallerId(String);
    /// # fn tools_for(_who: &str) -> Vec<(String, cli_framework::command::Command)> { vec![] }
    /// # let registry = CommandRegistry::new();
    /// let provider: McpDynamicToolProvider = Arc::new(|identity| {
    ///     // Downcast the opaque identity to the host's own type.
    ///     let who = identity
    ///         .and_then(|id| id.downcast_ref::<CallerId>().map(|c| c.0.clone()));
    ///     Box::pin(async move {
    ///         match who {
    ///             // A real provider awaits a database read here.
    ///             Some(who) => tools_for(&who),
    ///             None => Vec::new(),
    ///         }
    ///     })
    /// });
    /// let registry = McpToolRegistry::from_command_registry(&registry, "myapp")
    ///     .with_dynamic_tools(provider);
    /// ```
    #[cfg(feature = "mcp-server")]
    pub fn with_dynamic_tools(mut self, provider: McpDynamicToolProvider) -> Self {
        self.dynamic_tools = Some(provider);
        self
    }

    /// Run the installed authenticator (if any) against `headers`, returning
    /// the opaque identity it produces.
    ///
    /// Returns `None` when no authenticator is installed, or when the
    /// installed authenticator itself returns `None` for these headers.
    #[cfg(feature = "mcp-server")]
    fn authenticate(&self, headers: &http::HeaderMap) -> Option<Arc<dyn Any + Send + Sync>> {
        self.request_authenticator.as_ref().and_then(|f| f(headers))
    }

    /// Run the installed per-caller tool provider (if any) for `identity`.
    ///
    /// Returns an empty `Vec` when no provider is installed, without awaiting
    /// anything — the no-hook path is byte-identical to having no hook at all.
    #[cfg(feature = "mcp-server")]
    async fn dynamic_tools_for(
        &self,
        identity: Option<Arc<dyn Any + Send + Sync>>,
    ) -> Vec<(String, Command)> {
        match self.dynamic_tools.as_ref() {
            Some(provider) => provider(identity).await,
            None => Vec::new(),
        }
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

    /// The tool descriptors a caller with `identity` sees: the static set
    /// [`list_tools`](Self::list_tools) returns, followed by the commands an
    /// installed [`McpDynamicToolProvider`] supplies for that identity.
    ///
    /// With no provider installed this is exactly `list_tools()`, including
    /// its ordering. With one installed, the static block keeps the order
    /// `list_tools()` gave it and the per-caller block is appended after it in
    /// the order the provider returned, so the merge adds no nondeterminism of
    /// its own. Names already present in the static set — or repeated within
    /// the provider's own result — are skipped, so no name is described twice;
    /// each skip is reported at `tracing::warn!`, naming the tool.
    ///
    /// Descriptors on both paths come from the same
    /// `command_to_tool_descriptor_full`, so a per-caller tool's description
    /// and input schema are generated exactly as a static one's would be.
    ///
    /// This is a discovery surface, not an authorization one; see
    /// [`with_dynamic_tools`](Self::with_dynamic_tools).
    #[cfg(feature = "mcp-server")]
    pub async fn list_tools_for_identity(
        &self,
        identity: Option<Arc<dyn Any + Send + Sync>>,
    ) -> Vec<McpToolDescriptor> {
        let mut descriptors = self.list_tools();
        if self.dynamic_tools.is_none() {
            return descriptors;
        }
        // Two distinct reasons to drop a provided pair, warned about
        // separately: the name is already a static tool (static wins), or the
        // provider returned it twice in one result (first wins). Both are
        // provider bugs the consumer cannot otherwise see — a tool it believes
        // it published simply is not there — so neither is silent.
        let mut emitted: std::collections::HashSet<String> = std::collections::HashSet::new();
        for (name, cmd) in self.dynamic_tools_for(identity).await {
            if self.tools.contains_key(&name) {
                tracing::warn!(
                    "MCP per-caller tool '{}' collides with a statically registered command; \
                     the static command wins and the per-caller one is not listed",
                    name
                );
                continue;
            }
            if !emitted.insert(name.clone()) {
                tracing::warn!(
                    "MCP per-caller tool provider returned the name '{}' more than once in one \
                     result; the first pair wins and later ones are dropped",
                    name
                );
                continue;
            }
            descriptors.push(command_to_tool_descriptor_full(&name, &cmd));
        }
        descriptors
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
    telemetry: std::sync::Arc<dyn crate::telemetry::Telemetry + Send + Sync>,
    /// Opaque per-request identity, stashed by the MCP HTTP transport when a
    /// `McpRequestAuthenticator` is installed and produced a value for this
    /// call's headers. `None` under stdio, or when no authenticator is
    /// installed, or when the authenticator itself returned `None`.
    identity: Option<Arc<dyn Any + Send + Sync>>,
}
#[cfg(feature = "mcp-server")]
impl McpAppContext {
    fn new(
        telemetry: Option<std::sync::Arc<dyn crate::telemetry::Telemetry + Send + Sync>>,
        identity: Option<Arc<dyn Any + Send + Sync>>,
    ) -> Self {
        Self {
            buffer: std::sync::Mutex::new(Vec::new()),
            structured: std::sync::Mutex::new(None),
            telemetry: telemetry
                .unwrap_or_else(|| std::sync::Arc::new(crate::telemetry::NoopTelemetry)),
            identity,
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

    fn telemetry(&self) -> &dyn crate::telemetry::Telemetry {
        self.telemetry.as_ref()
    }

    fn opt_telemetry_arc(
        &self,
    ) -> Option<std::sync::Arc<dyn crate::telemetry::Telemetry + Send + Sync>> {
        Some(std::sync::Arc::clone(&self.telemetry))
    }

    fn opt_request_identity(&self) -> Option<std::sync::Arc<dyn Any + Send + Sync>> {
        self.identity.clone()
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

    // rmcp 2 `Tool` carries a per-tool `_meta` passthrough (`Tool::meta`,
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
                let mut resource = Resource::new(listing.uri, listing.name);
                resource.description = listing.description;
                resource.mime_type = listing.mime_type;
                resource
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
        context: RequestContext<RoleServer>,
    ) -> impl std::future::Future<Output = Result<ListToolsResult, ErrorData>> + Send + '_ {
        // Same per-request identity seam `call_tool` uses, for the same
        // reason: the advertised set and the callable set are derived from one
        // identity so they cannot drift. Under stdio there is no HTTP request,
        // so `identity` is `None` and an installed provider is invoked with
        // `None` (see `McpToolRegistry::with_dynamic_tools`).
        let registry = Arc::clone(&self.tool_registry);
        // With no provider installed this path must stay byte-identical to
        // what it was before per-caller tool sets existed, which includes
        // *not* running the consumer's authenticator: that closure is
        // consumer code and may log, emit metrics or consume a rate-limit
        // budget, so invoking it on a `tools/list` that cannot use its
        // result would be a new observable side effect.
        let identity = if registry.dynamic_tools.is_some() {
            context
                .extensions
                .get::<http::request::Parts>()
                .and_then(|parts| registry.authenticate(&parts.headers))
        } else {
            None
        };

        async move {
            let descriptors = registry.list_tools_for_identity(identity).await;
            let tools: Vec<Tool> = descriptors.iter().map(make_rmcp_tool).collect();
            Ok(ListToolsResult {
                tools,
                next_cursor: None,
                meta: Default::default(),
            })
        }
    }

    fn call_tool(
        &self,
        request: CallToolRequestParams,
        context: RequestContext<RoleServer>,
    ) -> impl std::future::Future<Output = Result<CallToolResult, ErrorData>> + Send + '_ {
        let tool_name = request.name.to_string();
        let arguments = request.arguments;
        let registry = Arc::clone(&self.tool_registry);
        let transport = self.transport;
        let serialize = self.stdio_serialize.as_ref().map(Arc::clone);

        // Per-request identity seam: on the HTTP transport, rmcp's
        // Streamable HTTP layer injects the raw `http::request::Parts` for
        // this call into `context.extensions` (see
        // `rmcp::transport::streamable_http_server::tower`). Under stdio
        // there is no HTTP request, so this is always `None` regardless of
        // whether an authenticator is installed — the accessor still
        // compiles and behaves (`request_identity() == None`).
        let identity = context
            .extensions
            .get::<http::request::Parts>()
            .and_then(|parts| registry.authenticate(&parts.headers));

        async move {
            if let Some(lock) = serialize {
                let _guard = lock.lock().await;
                dispatch_tool_call_spawned_with_identity(
                    registry, tool_name, arguments, transport, identity,
                )
                .await
            } else {
                dispatch_tool_call_spawned_with_identity(
                    registry, tool_name, arguments, transport, identity,
                )
                .await
            }
        }
    }
}

/// Dispatch a single MCP tool call to its backing command.
///
/// This is the MCP-surface counterpart to the CLI dispatch path: it resolves the
/// tool name to a command, validates arguments, applies the tool gate, and runs
/// the command against an `McpAppContext`. When the registry carries a telemetry
/// handle (see [`McpToolRegistry::with_telemetry`]), the call is wrapped in a
/// `cli.command` span tagged `cli.invocation.surface = "mcp"`.
#[cfg(feature = "mcp-server")]
pub async fn dispatch_tool_call(
    tool_registry: &McpToolRegistry,
    tool_name: &str,
    arguments: Option<JsonObject>,
    transport: McpTransportKind,
) -> Result<CallToolResult, ErrorData> {
    dispatch_tool_call_with_identity(tool_registry, tool_name, arguments, transport, None).await
}

/// Like [`dispatch_tool_call`], but threads a per-request opaque `identity`
/// (produced by an installed [`McpRequestAuthenticator`], see
/// [`McpToolRegistry::with_request_authenticator`]) into the tool's
/// [`McpAppContext`] for this call, readable inside the command's `execute`
/// closure via [`crate::app::RequestIdentityExt::request_identity`].
#[cfg(feature = "mcp-server")]
pub async fn dispatch_tool_call_with_identity(
    tool_registry: &McpToolRegistry,
    tool_name: &str,
    arguments: Option<JsonObject>,
    transport: McpTransportKind,
    identity: Option<Arc<dyn Any + Send + Sync>>,
) -> Result<CallToolResult, ErrorData> {
    use crate::command_surface::tool_bridge::{
        BridgeError, BridgeInput, BridgeInvocation, ConfirmationMode,
    };

    // Static set first: a name registered at construction always resolves to
    // its static command, whatever a per-caller provider returns for it
    // (documented precedence on `McpToolRegistry::with_dynamic_tools`). Only a
    // static miss consults the provider, and only for *this* request's
    // identity — nothing about the per-caller set is cached between requests,
    // which is what keeps this path and `tools/list` in agreement.
    let dynamic_cmd = match tool_registry.resolve_tool(tool_name) {
        Some(_) => None,
        None => tool_registry
            .dynamic_tools_for(identity.clone())
            .await
            .into_iter()
            .find(|(name, _)| name == tool_name)
            .map(|(_, cmd)| cmd),
    };
    let cmd = tool_registry
        .resolve_tool(tool_name)
        .or(dynamic_cmd.as_ref())
        .ok_or_else(|| {
            mcp_error(
                -32001,
                format!("MCP_CMD_NOT_FOUND: tool '{}' not registered", tool_name),
            )
        })?;

    let bridge = tool_registry.bridge_for_call(transport, tool_name);

    let arguments_value = arguments.map(Value::Object).unwrap_or(Value::Null);
    let mut ctx = McpAppContext::new(tool_registry.telemetry.clone(), identity);

    // The span is created here, outside `res`'s block, and kept alive via
    // `span.clone()` below rather than moved into `.instrument`: `status`
    // (mcp.session probe, PRD 336) is not known until the outcome is computed
    // further down, so the still-open span must survive past the `.await` to
    // receive it. Declaring it as `tracing::field::Empty` at creation and
    // recording it only once known avoids fabricating a status up front.
    #[cfg(feature = "telemetry")]
    let span = {
        let span = tracing::info_span!(
            "cli.command",
            "cli.command.path" = tool_name,
            "cli.invocation.surface" = "mcp",
            "cli.probe" = tracing::field::Empty,
            "mcp.tool" = tracing::field::Empty,
            "status" = tracing::field::Empty,
        );
        for kv in crate::telemetry::mcp_session_attrs(Some(tool_name), None) {
            span.record(kv.key.as_str(), kv.value.as_str().as_ref());
        }
        span
    };
    #[cfg(not(feature = "telemetry"))]
    let span = tracing::Span::none();

    let res = {
        use tracing::Instrument;
        bridge
            .invoke_structured(
                &mut ctx,
                BridgeInvocation {
                    command: cmd,
                    input: BridgeInput::Json(arguments_value),
                    confirmation: ConfirmationMode::NonInteractive,
                    mode: crate::command_surface::tool_bridge::BridgeMode::Mcp,
                },
            )
            .instrument(span.clone())
            .await
    };

    let outcome = match res {
        Ok(output) => {
            let text = if output.text.is_empty() {
                "OK"
            } else {
                &output.text
            };
            // CF-7: a command may attach a `structuredContent` value distinct
            // from the `content` text (e.g. server-rendered View HTML), kept out
            // of the model's text context.
            let mut result = CallToolResult::success(vec![ContentBlock::text(text)]);
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
    };

    // `mcp.session` probe (Task 20): one increment per dispatched call, tagged
    // with the tool name (bounded — it comes from the server's own declared
    // tool list) and a closed ok/error vocabulary. `ctx.telemetry()` is the
    // same handle `McpAppContext` already carries (falls back to a no-op when
    // no provider is configured), matching how `src/app/builder.rs`'s command
    // dispatch pairs a span with a counter at the same callsite.
    #[cfg(feature = "telemetry")]
    {
        use crate::app::AppContext as _;
        let status = if outcome.is_ok() { "ok" } else { "error" };
        for kv in crate::telemetry::mcp_session_attrs(None, Some(status)) {
            span.record(kv.key.as_str(), kv.value.as_str().as_ref());
        }
        ctx.telemetry()
            .counter(crate::telemetry::metrics::MCP_TOOL_CALLS)
            .add(
                1,
                &[
                    crate::telemetry::KeyValue::new("tool", tool_name.to_string()),
                    crate::telemetry::KeyValue::new("status", status),
                ],
            );
    }

    outcome
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
    dispatch_tool_call_spawned_with_identity(tool_registry, tool_name, arguments, transport, None)
        .await
}

/// Like [`dispatch_tool_call_spawned`], but threads a per-request opaque
/// `identity` through to [`dispatch_tool_call_with_identity`].
#[cfg(feature = "mcp-server")]
pub async fn dispatch_tool_call_spawned_with_identity(
    tool_registry: Arc<McpToolRegistry>,
    tool_name: String,
    arguments: Option<JsonObject>,
    transport: McpTransportKind,
    identity: Option<Arc<dyn Any + Send + Sync>>,
) -> Result<CallToolResult, ErrorData> {
    let handle = tokio::spawn(async move {
        dispatch_tool_call_with_identity(&tool_registry, &tool_name, arguments, transport, identity)
            .await
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

/// Like [`build_mcp_axum_router_with_resources`], but overrides the inbound
/// `Host`-header allowlist enforced by rmcp's Streamable HTTP transport. See
/// [`transport_http::mcp_axum_router_with_host_policy`] for the semantics of
/// `allowed_hosts` (`Some` = allow exactly these authorities, `None` =
/// disable Host validation — network-isolated deployments only).
#[cfg(feature = "mcp-server")]
pub fn build_mcp_axum_router_with_host_policy(
    registry: &CommandRegistry,
    app_name: &str,
    _path: &str,
    risk_policy: crate::security::CommandRiskPolicy,
    export_policy: McpToolExportPolicy,
    resource_registry: Arc<resources::ResourceRegistry>,
    allowed_hosts: Option<Vec<String>>,
) -> axum::Router {
    let tool_registry = Arc::new(
        McpToolRegistry::from_command_registry_with_policy(registry, app_name, export_policy)
            .with_risk_policy(risk_policy),
    );
    transport_http::mcp_axum_router_with_host_policy(
        tool_registry,
        resource_registry,
        allowed_hosts,
    )
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
        None,
        None,
        None,
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
    telemetry: Option<std::sync::Arc<dyn crate::telemetry::Telemetry + Send + Sync>>,
    request_authenticator: Option<McpRequestAuthenticator>,
    dynamic_tools: Option<McpDynamicToolProvider>,
) -> Result<()> {
    let mut tool_registry =
        McpToolRegistry::from_command_registry_with_policy(&registry, app_name, export_policy)
            .with_risk_policy(risk_policy);
    if let Some(gate) = gate {
        tool_registry = tool_registry.with_gate(gate);
    }
    if let Some(tel) = telemetry {
        tool_registry = tool_registry.with_telemetry(tel);
    }
    if let Some(authenticator) = request_authenticator {
        tool_registry = tool_registry.with_request_authenticator(authenticator);
    }
    if let Some(provider) = dynamic_tools {
        tool_registry = tool_registry.with_dynamic_tools(provider);
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
        None,
        None,
        None,
    )
    .await
}

/// Like [`serve_mcp_stdio_opts`], but threads a populated
/// [`resources::ResourceRegistry`] into the served handler so registered
/// `ui://…` resources are served over the stdio transport.
///
/// `request_authenticator`, if installed, is stored on the tool registry for
/// parity with the HTTP entry point, but is never invoked here: stdio has no
/// HTTP request to authenticate, so `request_identity()` remains `None`.
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
    telemetry: Option<std::sync::Arc<dyn crate::telemetry::Telemetry + Send + Sync>>,
    request_authenticator: Option<McpRequestAuthenticator>,
    dynamic_tools: Option<McpDynamicToolProvider>,
) -> anyhow::Result<()> {
    let mut tool_registry =
        McpToolRegistry::from_command_registry_with_policy(&registry, app_name, export_policy)
            .with_risk_policy(risk_policy);
    if let Some(gate) = gate {
        tool_registry = tool_registry.with_gate(gate);
    }
    if let Some(tel) = telemetry {
        tool_registry = tool_registry.with_telemetry(tel);
    }
    if let Some(authenticator) = request_authenticator {
        tool_registry = tool_registry.with_request_authenticator(authenticator);
    }
    if let Some(provider) = dynamic_tools {
        tool_registry = tool_registry.with_dynamic_tools(provider);
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
        let text = match &result.content[0] {
            rmcp::model::ContentBlock::Text(t) => t.text.clone(),
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
