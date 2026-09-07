use crate::ailoop::{AiloopClient, AiloopConfig};
use crate::app::context::AppContext;
use crate::app::dispatch::InvocationSurface;
use crate::app::module::Module;
use crate::app::AppMeta;
use crate::cli_output::HelpRenderer;
use crate::command::{Command, CommandRegistry, TypedArgs};
use crate::environment::EnvironmentVariableRegistry;
use crate::plugin::PluginRegistryManager;
use crate::spec::arg_spec::ArgSpec;
use crate::spec::command_tree::{CommandPath, EnvVarEntry, GroupMetadata};
use crate::spec::value::ArgValue;
use anyhow::Result;
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::PathBuf;
use std::sync::Arc;

/// Marker error for parse and usage failures.
///
/// When [`App::run`] receives this error from [`App::run_with_args`] it calls
/// `std::process::exit(2)` — the diagnostic message has already been printed
/// to stderr by [`DiagnosticReporter`][crate::app::diagnostic_reporter::DiagnosticReporter].
///
/// # Exit-code contract (spec 012 §R5)
///
/// - **Usage / parse errors → exit `2`**: unrecognized subcommand, missing required
///   argument, invalid Enum value, unsupported completion shell, unknown spec format,
///   unknown doctor check, validation failures (E003–E006).
/// - **Runtime errors → exit `1`**: agent/IO failures, `doctor` reporting health
///   problems (a successful diagnostic run that found errors is a runtime result).
/// - **`0`** remains success only.
#[derive(Debug, thiserror::Error)]
#[error("{0}")]
pub struct UsageError(pub String);

use std::sync::Mutex;

pub struct AppBuilder {
    command_registry: CommandRegistry,
    plugin_registry_manager: Option<PluginRegistryManager>,
    ailoop_config: Option<AiloopConfig>,
    plugin_registry_path: Option<PathBuf>,
    meta: Option<AppMeta>,
    app_name: &'static str,
    app_version: &'static str,
    app_git_sha_short: Option<&'static str>,
    risk_policy: crate::security::command_risk::CommandRiskPolicy,
    auto_register_completion: bool,
    builtin_command_namespace: CommandPath,
    global_flags: Vec<ArgSpec>,
    environment_variables: EnvironmentVariableRegistry,
    #[cfg(feature = "doctor")]
    doctor_checks: Vec<Arc<dyn crate::doctor::check::DoctorCheck>>,
    #[cfg(feature = "mcp-server")]
    mcp_export_policy: crate::mcp::McpToolExportPolicy,
    #[cfg(feature = "mcp-server")]
    mcp_tool_gate: Option<std::sync::Arc<dyn crate::security::ExecutionGate>>,
    #[cfg(feature = "mcp-server")]
    mcp_resource_registry: Option<std::sync::Arc<crate::mcp::resources::ResourceRegistry>>,
    #[cfg(feature = "mcp-server")]
    mcp_request_authenticator: Option<crate::mcp::McpRequestAuthenticator>,
    #[cfg(feature = "mcp-server")]
    auto_register_mcp: bool,
    #[cfg(feature = "chat")]
    chat_tool_policy: crate::command::chat::ChatToolPolicy,
    suggest_corrections: bool,
    #[cfg(feature = "auth")]
    token_provider: Option<Arc<dyn crate::auth::TokenProvider>>,
    telemetry_config: Option<crate::telemetry::TelemetryConfig>,
    /// The application's deployment shape (spec 025). Ungated: `Deployment`
    /// lives in `telemetry::axes`, which compiles unconditionally, and this
    /// field is read outside the `telemetry` feature too (it will gate
    /// startup behaviour PR7 adds). Defaults to `EndUser { privacy_url: None
    /// }`, the PRD's default for an app that never calls `with_deployment`.
    deployment: crate::telemetry::Deployment,
    /// Test-only override for where the `telemetry` settings file lives; see
    /// [`Self::with_telemetry_config_dir`].
    #[cfg(feature = "telemetry")]
    telemetry_store_dir: Option<PathBuf>,
    /// Transport defaults the author picked; the operator's
    /// `OTEL_EXPORTER_OTLP_*` variables win over both fields (spec 025).
    #[cfg(feature = "telemetry")]
    telemetry_defaults: crate::telemetry::TelemetryDefaults,
    /// Operational probes the author registered, in call order. Kept as the
    /// `'static` slices they arrived as: they are folded into one
    /// `ProbeRegistry` at build, which is where a malformed or colliding id
    /// becomes a build error.
    #[cfg(feature = "telemetry")]
    telemetry_ops: Vec<&'static [crate::telemetry::ProbeSpec]>,
    /// How the app answers "who is calling?"; see
    /// [`crate::telemetry::IdentityResolver`].
    #[cfg(feature = "telemetry")]
    telemetry_identity: Option<crate::telemetry::IdentityResolver>,
    /// Application attribute keys the author allowlisted.
    #[cfg(feature = "telemetry")]
    telemetry_attrs: Vec<String>,
    /// Extra never-list fragments the author added.
    #[cfg(feature = "telemetry")]
    telemetry_never: Vec<String>,
    /// The format the app's own configuration is stored in, captured from
    /// [`Self::with_config`] before its `ConfigOptions` moves into the
    /// registration closure. The framework-owned telemetry file follows it
    /// (spec 025: "the extension follows the app's configuration format"),
    /// so an app storing TOML gets `telemetry.toml`, not a lone JSON file.
    #[cfg(feature = "config")]
    config_format: crate::config::ConfigFormat,
    #[cfg(feature = "config")]
    config_backend: Option<Arc<dyn crate::config::ConfigBackend>>,
    #[cfg(feature = "config")]
    config_path: Option<PathBuf>,
    #[cfg(feature = "config")]
    config_registration: Option<Box<ConfigRegistrationFn>>,
    #[cfg(feature = "config")]
    config_manifest: Option<Arc<crate::config::manifest::ConfigManifest>>,
    #[cfg(feature = "config-managed")]
    policy_client: Option<Arc<crate::config::managed::PolicyClient>>,
}

/// Result of resolving a registered `with_config::<T>()` call against a
/// concrete backend: the type-erased [`crate::config::ConfigHandle`] wired
/// into `AppContext::opt_config_handle`, and the same store type-erased as
/// `Any` so [`App::config_store`] can downcast it back to `ConfigStore<T>`.
#[cfg(feature = "config")]
type ConfigResolution = (
    Arc<dyn crate::config::ConfigHandle>,
    Arc<dyn std::any::Any + Send + Sync>,
);

/// Closure captured by `with_config::<T>()`, monomorphized for that `T`, that
/// builds a `ConfigStore<T>` over whatever backend `build()` resolves and
/// runs resolution once. Boxed so `AppBuilder` (non-generic over `T`) can
/// hold one regardless of which `T` an application uses.
#[cfg(feature = "config")]
type ConfigRegistrationFn = dyn FnOnce(
        Arc<dyn crate::config::ConfigBackend>,
    ) -> Result<ConfigResolution, crate::config::ConfigError>
    + Send;

impl AppBuilder {
    pub fn new() -> Self {
        Self {
            command_registry: CommandRegistry::new(),
            plugin_registry_manager: None,
            ailoop_config: None,
            plugin_registry_path: None,
            meta: None,
            app_name: "unknown",
            app_version: "unknown",
            app_git_sha_short: None,
            risk_policy: crate::security::command_risk::CommandRiskPolicy::default(),
            auto_register_completion: true,
            builtin_command_namespace: CommandPath::default(),
            global_flags: Vec::new(),
            environment_variables: EnvironmentVariableRegistry::new(),
            #[cfg(feature = "doctor")]
            doctor_checks: Vec::new(),
            #[cfg(feature = "mcp-server")]
            mcp_export_policy: crate::mcp::McpToolExportPolicy::default(),
            #[cfg(feature = "mcp-server")]
            mcp_tool_gate: None,
            #[cfg(feature = "mcp-server")]
            mcp_resource_registry: None,
            #[cfg(feature = "mcp-server")]
            mcp_request_authenticator: None,
            #[cfg(feature = "mcp-server")]
            auto_register_mcp: true,
            #[cfg(feature = "chat")]
            chat_tool_policy: crate::command::chat::ChatToolPolicy::default(),
            suggest_corrections: true,
            #[cfg(feature = "auth")]
            token_provider: None,
            telemetry_config: None,
            deployment: crate::telemetry::Deployment::EndUser { privacy_url: None },
            #[cfg(feature = "telemetry")]
            telemetry_store_dir: None,
            #[cfg(feature = "telemetry")]
            telemetry_defaults: crate::telemetry::TelemetryDefaults::default(),
            #[cfg(feature = "telemetry")]
            telemetry_ops: Vec::new(),
            #[cfg(feature = "telemetry")]
            telemetry_identity: None,
            #[cfg(feature = "telemetry")]
            telemetry_attrs: Vec::new(),
            #[cfg(feature = "telemetry")]
            telemetry_never: Vec::new(),
            #[cfg(feature = "config")]
            config_format: crate::config::ConfigFormat::default(),
            #[cfg(feature = "config")]
            config_backend: None,
            #[cfg(feature = "config")]
            config_path: None,
            #[cfg(feature = "config")]
            config_registration: None,
            #[cfg(feature = "config")]
            config_manifest: None,
            #[cfg(feature = "config-managed")]
            policy_client: None,
        }
    }

    /// Enable or disable "Did you mean?" suggestions for unknown subcommands
    /// and flags. Default: `true`.
    pub fn suggest_corrections(mut self, enabled: bool) -> Self {
        self.suggest_corrections = enabled;
        self
    }

    /// Register a [`TokenProvider`][crate::auth::TokenProvider] that will be
    /// injected into every command dispatch via
    /// [`AppContext::opt_token_provider`][crate::app::AppContext::opt_token_provider].
    ///
    /// When set, the built-in `auth` command group (`auth login`, `auth logout`,
    /// `auth status`, `auth token`) is automatically registered.
    #[cfg(feature = "auth")]
    pub fn with_token_provider(mut self, provider: Arc<dyn crate::auth::TokenProvider>) -> Self {
        self.token_provider = Some(provider);
        self
    }

    /// Configure OpenTelemetry telemetry for this app.
    ///
    /// When set and `TelemetryConfig::is_active()` returns `true`, the framework will
    /// initialise an OTLP exporter on every `run()` call and attach a telemetry handle
    /// to the command dispatch context.
    pub fn with_telemetry(mut self, config: crate::telemetry::TelemetryConfig) -> Self {
        self.telemetry_config = Some(config);
        self
    }

    /// Select an explicit [`ConfigBackend`][crate::config::ConfigBackend] for the
    /// typed config registered via [`Self::with_config`].
    ///
    /// Takes precedence over [`Self::with_config_path`]. When neither this nor
    /// `with_config_path` is called, `build()` defaults to
    /// `FileBackend::for_app(app_name)`.
    #[cfg(feature = "config")]
    pub fn with_config_backend(mut self, backend: Arc<dyn crate::config::ConfigBackend>) -> Self {
        self.config_backend = Some(backend);
        self
    }

    /// Shorthand for `with_config_backend(Arc::new(FileBackend::new(path)))`.
    #[cfg(feature = "config")]
    pub fn with_config_path(mut self, path: impl Into<PathBuf>) -> Self {
        self.config_path = Some(path.into());
        self
    }

    /// Register a typed configuration `T`, its format, and its migrations.
    ///
    /// Causes `build()` to construct a `ConfigStore<T>` over the configured
    /// backend (see [`Self::with_config_backend`] / [`Self::with_config_path`])
    /// and run resolution exactly once, at the same point registry freezing
    /// happens. The resulting type-erased handle is what
    /// `AppContext::opt_config_handle` returns during dispatch; the typed
    /// resolved value is not threaded through `AppContext` at all — get it
    /// back via [`Self::build_with_config`] or, for reload/subscribe access,
    /// via [`App::config_store`] after building.
    #[cfg(feature = "config")]
    pub fn with_config<T>(mut self, options: crate::config::ConfigOptions<T>) -> Self
    where
        T: crate::config::VersionedConfig,
    {
        // Read the format out *before* the closure below moves `options`.
        // Nothing else can recover it afterwards, and the framework-owned
        // telemetry file needs it (spec 025).
        self.config_format = options.format;
        self.config_registration = Some(Box::new(move |backend| {
            let mut store = crate::config::ConfigStore::<T>::new(
                backend,
                options.format,
                options.current_version,
            );
            for (from_version, migration) in options.migrations {
                store = store.with_migration(from_version, move |value| migration(value));
            }
            store.resolve()?;
            let store = Arc::new(store);
            let handle: Arc<dyn crate::config::ConfigHandle> = store.clone();
            let erased: Arc<dyn std::any::Any + Send + Sync> = store;
            Ok((handle, erased))
        }));
        self
    }

    /// Register the application's declared configuration surface (spec 021,
    /// "Config manifest") for the built-in `config` command group.
    ///
    /// Deliberately independent of [`Self::with_config`]: the manifest
    /// describes what fields *exist* and their policy flags, while
    /// `with_config::<T>()` wires the *store* that backs the `config file`
    /// layer during resolution. An application typically calls both —
    /// `with_config_manifest(T::config_manifest())` (from
    /// `#[derive(ConfigManifest)]`, or a hand-authored [`ConfigManifest`])
    /// alongside `with_config::<T>(options)` — but only this call is what
    /// makes `build()` auto-register the `config` command group (`config
    /// show`, `config manifest`, and — under the `config-managed` feature —
    /// `config profile`/`config refresh`; see spec 021, "Command surface").
    ///
    /// [`ConfigManifest`]: crate::config::manifest::ConfigManifest
    #[cfg(feature = "config")]
    pub fn with_config_manifest(
        mut self,
        manifest: crate::config::manifest::ConfigManifest,
    ) -> Self {
        self.config_manifest = Some(Arc::new(manifest));
        self
    }

    /// Register a [`PolicyClient`][crate::config::managed::PolicyClient] for
    /// the built-in `config profile`/`config refresh` commands.
    ///
    /// The client's base URL, target app id, and the [`TokenProvider`] it
    /// authenticates with are the application's own concern to construct
    /// (spec 021, "Bootstrap": the service address resolves from
    /// environment, local config, or a builder default, and is structurally
    /// excluded from both server trees) — `cli-framework` only stores the
    /// already-built client and hands it to the command group through
    /// [`AppContext::opt_policy_client`][crate::app::AppContext::opt_policy_client].
    /// Without this call, `config profile`/`config refresh` report
    /// [`CFG002`][crate::parser::error_codes::CFG002] ("not managed") rather
    /// than failing to register at all — `config show`/`config manifest`
    /// remain available from [`Self::with_config_manifest`] alone.
    ///
    /// [`TokenProvider`]: crate::auth::TokenProvider
    #[cfg(feature = "config-managed")]
    pub fn with_policy_client(mut self, client: Arc<crate::config::managed::PolicyClient>) -> Self {
        self.policy_client = Some(client);
        self
    }

    /// Resolve the configured backend (explicit backend, explicit path, or
    /// `FileBackend::for_app(app_name)`) and, if [`Self::with_config`] was
    /// called, run its registration closure against it.
    #[cfg(feature = "config")]
    fn resolve_config(&mut self) -> Result<Option<ConfigResolution>> {
        let Some(registration) = self.config_registration.take() else {
            return Ok(None);
        };
        let backend: Arc<dyn crate::config::ConfigBackend> =
            if let Some(b) = self.config_backend.take() {
                b
            } else if let Some(p) = self.config_path.take() {
                Arc::new(crate::config::FileBackend::new(p))
            } else {
                Arc::new(
                    crate::config::FileBackend::for_app(self.app_name)
                        .map_err(|e| anyhow::anyhow!("{e}"))?,
                )
            };
        registration(backend)
            .map(Some)
            .map_err(|e| anyhow::anyhow!("{e}"))
    }

    /// Disable auto-registration of the built-in `completion` command.
    pub fn without_completion(mut self) -> Self {
        self.auto_register_completion = false;
        self
    }

    /// Place the framework-provided `spec` and `completion` commands below a
    /// namespace instead of registering them at the root.
    ///
    /// The default namespace is empty, preserving the root-level `spec` and
    /// `completion` paths. Register the namespace as a command group when it
    /// needs a custom summary, category, or root-help position.
    ///
    /// ```no_run
    /// # use cli_framework::prelude::*;
    /// let builder = AppBuilder::new()
    ///     .with_builtin_command_namespace(&CommandPath::root_for("cli"));
    /// // Built-ins will be registered as `cli/spec` and `cli/completion`.
    /// ```
    pub fn with_builtin_command_namespace(mut self, namespace: &CommandPath) -> Self {
        self.builtin_command_namespace = namespace.clone();
        self
    }

    /// Set the preferred order of categorized root-help sections.
    ///
    /// Unlisted sections follow the explicitly ordered sections in
    /// alphabetical order. The fallback `Other` section remains last.
    pub fn with_help_section_order(mut self, sections: &[&str]) -> Self {
        self.command_registry.set_help_section_order(sections);
        self
    }

    /// Suppress the built-in `mcp` command group (i.e. `mcp serve`).
    ///
    /// Use this when the binary exposes its MCP surface through a different
    /// mechanism (for example, a mounted router on an existing HTTP server via
    /// `build_mcp_axum_router`) and the standalone `mcp serve` subcommand would
    /// be confusing or redundant.
    #[cfg(feature = "mcp-server")]
    pub fn without_mcp(mut self) -> Self {
        self.auto_register_mcp = false;
        self
    }

    /// The configured application name (set by [`Self::with_version`]).
    ///
    /// MCP tool names are derived as `{app_name}_{command_path}`; embedders that
    /// register commands which reference *other* tools by name (e.g. a UI shim
    /// that calls an app-only bridge tool) read this to compute those names.
    pub fn app_name(&self) -> &str {
        self.app_name
    }

    /// Read-only access to the command registry accumulated so far.
    ///
    /// Useful to embedders that want to build an in-process
    /// [`McpToolRegistry`](crate::mcp::McpToolRegistry) for testing tool dispatch
    /// without standing up a transport.
    pub fn command_registry(&self) -> &CommandRegistry {
        &self.command_registry
    }

    /// Read-only access to the MCP resource registry supplied via
    /// [`Self::with_mcp_resource_registry`], if any.
    #[cfg(feature = "mcp-server")]
    pub fn mcp_resource_registry(
        &self,
    ) -> Option<&std::sync::Arc<crate::mcp::resources::ResourceRegistry>> {
        self.mcp_resource_registry.as_ref()
    }

    /// Add a global flag that applies to all commands.
    pub fn global_flag(mut self, spec: ArgSpec) -> Self {
        self.global_flags.push(spec);
        self
    }

    /// Declare an application-level environment variable for root help.
    ///
    /// Variables declared by individual [`CommandSpec`](crate::spec::CommandSpec)
    /// values are collected automatically when the app is built.
    pub fn register_env_var(mut self, entry: EnvVarEntry) -> Result<Self> {
        self.environment_variables.register(entry)?;
        Ok(self)
    }

    /// Set the MCP export policy used when `--mcp-serve` starts the embedded server.
    /// Default: `McpToolExportPolicy::AllCommands` (backward compatible).
    #[cfg(feature = "mcp-server")]
    pub fn with_mcp_export_policy(mut self, policy: crate::mcp::McpToolExportPolicy) -> Self {
        self.mcp_export_policy = policy;
        self
    }

    /// Set the tool-exposure policy for the built-in chat agent.
    ///
    /// Default: [`crate::command::chat::ChatToolPolicy::All`] (all commands exposed — backward compatible).
    /// Use [`crate::command::chat::ChatToolPolicy::UseCommandFlag`] to honor per-command `expose_chat` flags.
    #[cfg(feature = "chat")]
    pub fn with_chat_tool_policy(mut self, policy: crate::command::chat::ChatToolPolicy) -> Self {
        self.chat_tool_policy = policy;
        self
    }

    /// Configure an optional pre-execution gate for MCP tool calls.
    ///
    /// When unset, MCP behavior remains backward compatible (no gate).
    #[cfg(feature = "mcp-server")]
    pub fn with_mcp_tool_gate(
        mut self,
        gate: std::sync::Arc<dyn crate::security::ExecutionGate>,
    ) -> Self {
        self.mcp_tool_gate = Some(gate);
        self
    }

    /// Supply a populated [`crate::mcp::resources::ResourceRegistry`] whose
    /// `ui://…` resources the auto-registered `mcp serve` command will serve
    /// (over both the stdio and HTTP transports) via `resources/list` and
    /// `resources/read`.
    ///
    /// When unset, MCP serves a tools-only server (backward compatible).
    ///
    /// # Example
    ///
    /// ```rust,no_run
    /// # use cli_framework::app::AppBuilder;
    /// # use cli_framework::mcp::resources::{ResourceRegistry, UiResource};
    /// # use std::sync::Arc;
    /// let mut resources = ResourceRegistry::new();
    /// resources.register_static(
    ///     "ui://app/index.html",
    ///     "App shell",
    ///     UiResource::html("<!doctype html><title>App</title>"),
    /// );
    /// let app = AppBuilder::new()
    ///     .with_version("myapp", "0.1.0")
    ///     .with_mcp_resource_registry(Arc::new(resources));
    /// ```
    #[cfg(feature = "mcp-server")]
    pub fn with_mcp_resource_registry(
        mut self,
        resource_registry: std::sync::Arc<crate::mcp::resources::ResourceRegistry>,
    ) -> Self {
        self.mcp_resource_registry = Some(resource_registry);
        self
    }

    /// Install a per-request identity hook for the auto-registered `mcp serve`
    /// HTTP transport.
    ///
    /// cli-framework never parses bearer tokens, validates JWTs, or knows
    /// about JWKS/OIDC — it stays unopinionated about authentication. Instead,
    /// supply a closure that maps the incoming HTTP request's headers to an
    /// opaque, type-erased identity value (for example, a downstream
    /// product's own `SecurityContext`, built from a validated Bearer token).
    /// The MCP HTTP transport invokes this closure once per HTTP request and
    /// stashes the returned value so a tool's `execute` closure can read it
    /// back via `ctx.request_identity::<T>()`
    /// ([`crate::app::RequestIdentityExt`], blanket-implemented for every
    /// [`AppContext`]).
    ///
    /// Opt-in: when unset, `request_identity` always returns `None` and
    /// behavior is unchanged from before this hook existed. The hook only
    /// fires on the HTTP transport — under stdio there is no HTTP request to
    /// authenticate, so the identity remains `None` there regardless.
    ///
    /// Out of scope (a separate future spec): OAuth resource-server metadata,
    /// `WWW-Authenticate` challenges, JWKS fetching, or token validation.
    /// This hook is only the headers → opaque-identity plumbing seam; the
    /// host owns all token semantics.
    ///
    /// # Example
    ///
    /// ```rust,no_run
    /// # use cli_framework::app::AppBuilder;
    /// # use std::sync::Arc;
    /// struct CallerId(String);
    ///
    /// let app = AppBuilder::new()
    ///     .with_version("myapp", "0.1.0")
    ///     .with_mcp_request_authenticator(Arc::new(|headers: &http::HeaderMap| {
    ///         let token = headers.get(http::header::AUTHORIZATION)?.to_str().ok()?;
    ///         let token = token.strip_prefix("Bearer ")?;
    ///         Some(Arc::new(CallerId(token.to_string())) as Arc<dyn std::any::Any + Send + Sync>)
    ///     }));
    /// ```
    #[cfg(feature = "mcp-server")]
    pub fn with_mcp_request_authenticator(
        mut self,
        authenticator: crate::mcp::McpRequestAuthenticator,
    ) -> Self {
        self.mcp_request_authenticator = Some(authenticator);
        self
    }

    /// Override the default (all-Safe) command risk policy.
    pub fn with_risk_policy(
        mut self,
        policy: crate::security::command_risk::CommandRiskPolicy,
    ) -> Self {
        self.risk_policy = policy;
        self
    }

    /// Register a typed command using derive-generated `TypedArgs`.
    ///
    /// The spec is taken from `T::command_spec()` and the handler receives a
    /// fully-validated, infallibly-extracted `T` instance.
    pub fn register<T, F, Fut>(mut self, path: CommandPath, handler: F) -> Result<Self>
    where
        T: TypedArgs,
        F: Fn(&mut dyn AppContext, T) -> Fut + Send + Sync + Clone + 'static,
        Fut: std::future::Future<Output = anyhow::Result<()>> + Send + 'static,
    {
        let spec = Arc::new(T::command_spec());
        let id: Arc<str> = Arc::from(path.leaf().unwrap_or(""));
        let handler = Arc::new(handler);
        let command = Command {
            id,
            spec,
            validator: None,
            expose_mcp: true,
            expose_chat: true,
            meta: None,
            visibility: None,
            execute: Arc::new(move |ctx, args| {
                let typed = T::from_arg_value_map(&args);
                let h = Arc::clone(&handler);
                Box::pin(async move { h(ctx, typed).await })
            }),
        };
        self.command_registry
            .register_at(&path, command)
            .map_err(|e| anyhow::anyhow!("{}", e))?;
        Ok(self)
    }

    /// Register a root-level command. Returns `Err` if the command ID is already occupied
    /// or an alias conflicts with an existing registration.
    pub fn register_command(mut self, command: Command) -> Result<Self> {
        let path = CommandPath::root_for(&command.id);
        self.command_registry
            .register_at(&path, command)
            .map_err(|e| anyhow::anyhow!("{}", e))?;
        Ok(self)
    }

    /// Register a command at an arbitrary `CommandPath`.
    pub fn register_command_at(mut self, path: &CommandPath, command: Command) -> Result<Self> {
        self.command_registry
            .register_at(path, command)
            .map_err(|e| anyhow::anyhow!("{}", e))?;
        Ok(self)
    }

    /// Register a command group (no command, just metadata).
    pub fn register_group(mut self, path: &CommandPath, metadata: GroupMetadata) -> Result<Self> {
        self.command_registry
            .register_group(path, metadata)
            .map_err(|e| anyhow::anyhow!("{}", e))?;
        Ok(self)
    }

    pub fn register_module<M: Module>(mut self, module: M) -> Result<Self> {
        module.register(&mut self)?;
        Ok(self)
    }

    pub fn with_ailoop_config(mut self, config: AiloopConfig) -> Self {
        self.ailoop_config = Some(config);
        self
    }

    pub fn with_ailoop_channel(self, channel: &str) -> Self {
        let config = AiloopConfig {
            channel: channel.to_string(),
            server_url: None,
            default_timeout_seconds: 300,
        };
        self.with_ailoop_config(config)
    }

    pub fn with_plugin_registry_path(mut self, path: PathBuf) -> Self {
        self.plugin_registry_path = Some(path.clone());
        self.plugin_registry_manager = Some(PluginRegistryManager::new(path));
        self
    }

    pub fn with_meta(mut self, meta: AppMeta) -> Self {
        self.meta = Some(meta);
        self
    }

    pub fn with_version(mut self, name: &'static str, version: &'static str) -> Self {
        self.app_name = name;
        self.app_version = version;
        self
    }

    /// Declare this application's deployment shape (spec 025).
    ///
    /// `Deployment::EndUser` (the default — an app that never calls this
    /// still gets it) is a program a person installs and runs themselves; it
    /// is what auto-registers the built-in `telemetry` command group so that
    /// person has somewhere to see and change what is sent. `Deployment::
    /// Service` is a program an operator runs as a server, where the
    /// telemetry level is a config decision made the same way every other
    /// config decision is, and a runtime toggle command would be a second
    /// source of truth `telemetry status` on one replica could not answer
    /// for the fleet — so no command group is registered there at all.
    pub fn with_deployment(mut self, deployment: crate::telemetry::Deployment) -> Self {
        self.deployment = deployment;
        self
    }

    /// The deployment shape configured via [`Self::with_deployment`].
    pub fn deployment(&self) -> &crate::telemetry::Deployment {
        &self.deployment
    }

    /// Build this app against a bare [`TestContext`], returning the build
    /// error rather than panicking.
    ///
    /// The fallible form exists because several build-time decisions are
    /// *supposed* to fail — a malformed operational probe id, an app that
    /// owns a top-level `telemetry` key — and a test that asserts on those
    /// needs the error, not a panic.
    #[doc(hidden)]
    pub fn try_build_for_test(self) -> Result<App<TestContext>> {
        self.build(TestContext)
    }

    /// Build this app against a bare [`TestContext`], panicking on failure.
    #[doc(hidden)]
    pub fn build_for_test(self) -> App<TestContext> {
        self.try_build_for_test().expect("the test app builds")
    }

    /// The transport defaults this app ships with (spec 025).
    ///
    /// Both `endpoint` and `headers` are *defaults*: whoever runs the process
    /// overrides either with `OTEL_EXPORTER_OTLP_ENDPOINT` /
    /// `OTEL_EXPORTER_OTLP_HEADERS`. Headers are a
    /// [`SecretString`][crate::SecretString] and never reach the config tree,
    /// so a bearer token cannot be read back out of `telemetry status`,
    /// roamed to another machine, or printed by a `Debug` line.
    ///
    /// ```rust,no_run
    /// use cli_framework::app::AppBuilder;
    /// use cli_framework::{Deployment, TelemetryDefaults};
    ///
    /// let builder = AppBuilder::new()
    ///     .with_version("demo", "0.1.0")
    ///     .with_deployment(Deployment::Service)
    ///     .with_telemetry_defaults(TelemetryDefaults {
    ///         endpoint: Some("http://collector:4318".into()),
    ///         ..Default::default()
    ///     });
    /// ```
    #[cfg(feature = "telemetry")]
    pub fn with_telemetry_defaults(
        mut self,
        defaults: crate::telemetry::TelemetryDefaults,
    ) -> Self {
        self.telemetry_defaults = defaults;
        self
    }

    /// Register operational probes of the application's own.
    ///
    /// The slice is `'static` because a probe is a *declaration*, like a
    /// command: `telemetry info` lists it, the config manifest grows a
    /// `telemetry.<id>.enabled` switch for it, and both need the summary
    /// text to outlive any one invocation. Ids are validated at build —
    /// a malformed id, one that collides with a reserved first segment, or a
    /// duplicate fails [`Self::build`] rather than being silently dropped,
    /// because a probe that vanished quietly is a probe whose data never
    /// arrives and nobody notices.
    ///
    /// Call it more than once to register several groups; they are folded
    /// into one registry alongside the framework's built-in probes.
    #[cfg(feature = "telemetry")]
    pub fn with_telemetry_ops(mut self, probes: &'static [crate::telemetry::ProbeSpec]) -> Self {
        self.telemetry_ops.push(probes);
        self
    }

    /// Teach the app how to answer "who is calling?".
    ///
    /// A closure, not a value: identity is not known at build time — a CLI
    /// resolves it after authentication and a service resolves it per
    /// request. The framework calls it only when the resolved
    /// [`Attribution`][crate::telemetry::Attribution] permits attaching an
    /// identity at all.
    #[cfg(feature = "telemetry")]
    pub fn with_telemetry_identity(mut self, resolver: crate::telemetry::IdentityResolver) -> Self {
        self.telemetry_identity = Some(resolver);
        self
    }

    /// Allowlist application attribute keys.
    ///
    /// Framework attributes carry their own minimum telemetry level; an
    /// application attribute has none, so it is dropped at the export
    /// boundary unless its exact key appears here. The never-list still wins:
    /// allowlisting `app.api_key` does not bring it back.
    #[cfg(feature = "telemetry")]
    pub fn with_telemetry_attrs(mut self, keys: Vec<String>) -> Self {
        self.telemetry_attrs = keys;
        self
    }

    /// Add fragments to the never-list, which no telemetry level and no
    /// allowlist can override.
    ///
    /// The framework's own fragments (`password`, `secret`, `token`,
    /// `authorization`, `cookie`, `api_key`) always apply; these are extra
    /// ones an app knows about its own domain, such as `employee_id`.
    #[cfg(feature = "telemetry")]
    pub fn with_telemetry_never(mut self, fragments: Vec<String>) -> Self {
        self.telemetry_never = fragments;
        self
    }

    /// The format the app's own configuration is stored in, captured from
    /// [`Self::with_config`]. Defaults to
    /// [`ConfigFormat::default`][crate::config::ConfigFormat] for an app that
    /// declares no configuration.
    #[cfg(feature = "config")]
    pub fn config_format(&self) -> crate::config::ConfigFormat {
        self.config_format
    }

    /// Point telemetry's settings file at `dir` instead of the platform
    /// configuration directory. Test-only: an application has no reason to move
    /// a person's consent file somewhere unexpected.
    #[cfg(feature = "telemetry")]
    #[doc(hidden)]
    pub fn with_telemetry_config_dir(mut self, dir: impl Into<PathBuf>) -> Self {
        self.telemetry_store_dir = Some(dir.into());
        self
    }

    /// Opt-in build metadata to include a short git commit id in version output.
    ///
    /// - `None` clears the value.
    /// - Empty / whitespace-only strings are treated as `None`.
    /// - Invalid values are omitted and a warning is logged (ERR_VERSION_SHA_001).
    pub fn with_git_sha_short(mut self, sha: Option<&'static str>) -> Self {
        self.app_git_sha_short = crate::app::version::sanitize_git_sha_short(sha);
        self
    }

    #[cfg(feature = "doctor")]
    pub fn register_doctor_checks(
        mut self,
        checks: Vec<Arc<dyn crate::doctor::check::DoctorCheck>>,
    ) -> Self {
        self.doctor_checks.extend(checks);
        self
    }

    #[cfg(feature = "doctor")]
    pub(crate) fn push_doctor_checks(
        &mut self,
        checks: Vec<Arc<dyn crate::doctor::check::DoctorCheck>>,
    ) {
        self.doctor_checks.extend(checks);
    }

    /// Fold the framework's built-in probes together with every slice the
    /// author registered through [`Self::with_telemetry_ops`].
    ///
    /// A malformed id, a reserved first segment or a duplicate is returned as
    /// an error rather than skipped: a probe that disappeared quietly is a
    /// probe whose data never arrives and nobody finds out until they go
    /// looking for it.
    #[cfg(feature = "telemetry")]
    fn telemetry_registry(&self) -> Result<crate::telemetry::ProbeRegistry> {
        let mut registry = crate::telemetry::ProbeRegistry::with_builtins();
        for slice in &self.telemetry_ops {
            for spec in slice.iter() {
                registry.register(*spec)?;
            }
        }
        Ok(registry)
    }

    /// The one telemetry resolution this process makes (spec 025).
    ///
    /// Pure apart from reading the environment, which is exactly the layer
    /// this step is responsible for: the author's `TelemetryDefaults` supply
    /// the transport defaults, and whoever runs the process overrides either
    /// with `OTEL_EXPORTER_OTLP_ENDPOINT` / `OTEL_EXPORTER_OTLP_HEADERS`.
    /// Every *decision* — the deployment default, the end-user clamp, the
    /// kill switches — belongs to
    /// [`resolve_policy`][crate::telemetry::resolve_policy] and is made
    /// there.
    ///
    /// The stored settings file is deliberately not read here, and neither
    /// is the environment layer. Both belong to
    /// [`run_startup`](crate::telemetry::run_startup), which opens the
    /// framework-owned store on the first command and folds it, the
    /// `<APP>_TELEMETRY_*` variables and the published manifest into exactly
    /// these inputs before resolving once. Returning the *inputs* rather
    /// than a resolved policy is what makes that possible: a policy cannot
    /// be re-layered, only replaced.
    #[cfg(feature = "telemetry")]
    fn telemetry_inputs(
        &self,
        registry: crate::telemetry::ProbeRegistry,
    ) -> crate::telemetry::TelemetryInputs {
        use crate::config::resolution::Layer;

        let app = self.meta.as_ref().map(|m| m.name).unwrap_or(self.app_name);
        let read_env = |key: &str| std::env::var(key).ok().filter(|v| !v.is_empty());

        let (endpoint, endpoint_source) = match read_env("OTEL_EXPORTER_OTLP_ENDPOINT") {
            Some(from_env) => (Some(from_env), Some(Layer::Environment)),
            None => {
                let author = self.telemetry_defaults.endpoint.clone();
                let source = author.as_ref().map(|_| Layer::Default);
                (author, source)
            }
        };
        let headers = match read_env("OTEL_EXPORTER_OTLP_HEADERS") {
            Some(from_env) => Some(crate::SecretString::new(from_env)),
            None => self.telemetry_defaults.headers.clone(),
        };

        crate::telemetry::TelemetryInputs {
            app: app.to_string(),
            deployment: self.deployment.clone(),
            endpoint,
            endpoint_source,
            headers,
            session_id: uuid::Uuid::new_v4().to_string(),
            kill_switch: crate::telemetry::detect_kill_switch(app, &|k| std::env::var(k).ok()),
            registry,
            app_attr_allowlist: self.telemetry_attrs.clone(),
            extra_never: self.telemetry_never.clone(),
            ..Default::default()
        }
    }

    /// The one published manifest (spec 025).
    ///
    /// The framework generates the `telemetry` section from the resolved
    /// probe registry and inserts it into the application's own manifest. An
    /// application that publishes no manifest of its own still gets one, so
    /// the telemetry tree is administrable everywhere; its own `config`
    /// command group is still not auto-registered, because that decision is
    /// made from the *app-declared* manifest above.
    ///
    /// An app that owns a top-level `telemetry` key fails the build here,
    /// rather than having its section silently shadowed.
    #[cfg(feature = "telemetry")]
    fn published_telemetry_manifest(
        &self,
        registry: &crate::telemetry::ProbeRegistry,
    ) -> Result<crate::config::manifest::ConfigManifest> {
        let default_endpoint = self.telemetry_defaults.endpoint.as_deref();
        match self.config_manifest.as_deref() {
            Some(app_manifest) => Ok(crate::telemetry::merge_telemetry_section(
                app_manifest.clone(),
                registry,
                default_endpoint,
            )?),
            None => Ok(crate::telemetry::telemetry_only_manifest(
                self.app_name,
                registry,
                default_endpoint,
            )),
        }
    }

    pub fn build<C: AppContext + 'static>(mut self, ctx: C) -> Result<App<C>> {
        // Resolve config first (mirrors registry freezing / telemetry config capture
        // below in spirit: a framework-owned service is finalized exactly once here).
        #[cfg(feature = "config")]
        let config_resolved = self.resolve_config()?;

        // Spec 025: one telemetry resolution and one published manifest per
        // process, both settled before a single command group is registered
        // so that a malformed operational probe id fails the build outright
        // instead of after half a command tree has been assembled.
        #[cfg(feature = "telemetry")]
        let telemetry_inputs = {
            let registry = self.telemetry_registry()?;
            self.telemetry_inputs(registry)
        };
        // Resolved here as well as in `run_startup`, and deliberately: the
        // `telemetry` command group, the doctor and every span attribute
        // read `App::telemetry_policy`, and an app that is built but never
        // run -- which is most of this crate's own test suite -- still has
        // to be able to answer what it would send. Startup replaces it with
        // the resolution that folded in the stored consent.
        #[cfg(feature = "telemetry")]
        let telemetry_policy = Arc::new(crate::telemetry::resolve_policy(telemetry_inputs.clone()));
        #[cfg(feature = "telemetry")]
        let published_manifest =
            Arc::new(self.published_telemetry_manifest(&telemetry_inputs.registry)?);
        // The one place the settings file's location is decided. The
        // `telemetry` command group and the startup sequence must open the
        // same file: `telemetry set usage` writing one file while startup
        // reads another is a consent bug, not a path bug.
        #[cfg(feature = "telemetry")]
        let telemetry_store = crate::telemetry::TelemetryStoreLocation {
            dir: self.telemetry_store_dir.clone(),
            // Spec 025: the extension follows the app's own configuration
            // format, so a TOML app gets `telemetry.toml` rather than one
            // lone JSON file in an otherwise-TOML directory.
            format: self.config_format,
        };

        let ailoop_client = if let Some(config) = self.ailoop_config {
            Some(AiloopClient::with_config(config)?)
        } else {
            None
        };

        let plugin_registry_manager = self.plugin_registry_manager;

        #[cfg(feature = "chat")]
        {
            if self.command_registry.get("chat").is_none() {
                let registry_snapshot = Arc::new(self.command_registry.clone());
                let chat_command = crate::command::create_chat_command(
                    registry_snapshot,
                    self.risk_policy.clone(),
                    ailoop_client.clone().map(Arc::new),
                    self.app_name,
                    self.chat_tool_policy.clone(),
                );
                self.command_registry.register(chat_command);
            } else {
                tracing::warn!("'chat' command already registered; skipping built-in chat command");
            }
        }

        #[cfg(feature = "doctor")]
        {
            if !self.doctor_checks.is_empty() {
                if self.command_registry.get("doctor").is_none() {
                    let cmd = crate::doctor::command::create_doctor_command(std::mem::take(
                        &mut self.doctor_checks,
                    ));
                    self.command_registry.register(cmd);
                } else {
                    tracing::warn!(
                        "'doctor' command already registered; skipping auto-registration from doctor_checks"
                    );
                }
            }
        }

        // Auto-register `auth` command group when a provider is configured.
        #[cfg(feature = "auth")]
        if self.token_provider.is_some() {
            if self.command_registry.get("auth").is_none() {
                crate::auth::commands::register_auth_commands(
                    &mut self.command_registry,
                    self.app_name,
                )?;
            } else {
                tracing::warn!(
                    "'auth' command already registered; skipping built-in auth commands"
                );
            }
        }

        // Auto-register `config` command group (spec 021, "Command surface")
        // once an application has declared a config manifest — the whole
        // group lives behind `config-managed` because `config profile` /
        // `config refresh` are the pieces that actually need a `PolicyClient`
        // (`config show` / `config manifest` only need the plain-`config`
        // manifest + resolver, but there is no separate opt-in for a
        // narrower group; see `crate::config::commands` for the per-subcommand
        // gating on the code side).
        #[cfg(feature = "config-managed")]
        if self.config_manifest.is_some() {
            if self.command_registry.get("config").is_none() {
                crate::config::commands::register_config_commands(
                    &mut self.command_registry,
                    self.app_name,
                )?;
            } else {
                tracing::warn!(
                    "'config' command already registered; skipping built-in config commands"
                );
            }
        }

        // Auto-register `telemetry` command group (spec 025) on an EndUser
        // deployment only. A service's telemetry level is an operator's
        // config decision, made the same way every other config decision is
        // made; a runtime toggle command there would be a second source of
        // truth that `telemetry status` on one replica could not answer for
        // the fleet, so no group is registered for `Deployment::Service`.
        #[cfg(feature = "telemetry")]
        if self.deployment.is_end_user() {
            // `CommandRegistry::get` reads only `tree_commands`, but
            // `register_telemetry_commands` registers a *group*, and
            // `register_group` collides on either map. An app whose own
            // `telemetry` surface is a group -- the usual shape, because that
            // is what `myapp telemetry export` is -- would slip past a
            // `get`-only guard and then fail the build with a bare
            // "command path 'telemetry' is already occupied". Check both
            // namespaces so either shape reaches the same stand-down path.
            let already_owned = self.command_registry.get("telemetry").is_some()
                || self
                    .command_registry
                    .group_metadata_for("telemetry")
                    .is_some();
            if !already_owned {
                crate::telemetry::commands::register_telemetry_commands(
                    &mut self.command_registry,
                    self.app_name,
                    telemetry_store.clone(),
                )?;
            } else {
                tracing::warn!(
                    "'telemetry' command already registered; skipping built-in telemetry commands"
                );
            }
        }

        let builtin_namespace_is_command = !self.builtin_command_namespace.0.is_empty()
            && self
                .command_registry
                .resolve(&self.builtin_command_namespace)
                .is_some();
        if builtin_namespace_is_command {
            return Err(anyhow::anyhow!(
                "{}",
                crate::command::registry::RegistrationError::Collision {
                    path: self.builtin_command_namespace.to_path_string(),
                }
            ));
        }

        // Auto-register built-in `spec` command (always-on, no feature gate).
        let spec_path = self
            .builtin_command_namespace
            .push("spec")
            .expect("built-in command id is a valid path segment");
        if self.command_registry.resolve(&spec_path).is_none() {
            let spec_cmd = crate::command_surface::command::create_spec_command(
                self.app_name,
                self.app_version,
            );
            self.command_registry
                .register_at(&spec_path, spec_cmd)
                .map_err(|e| anyhow::anyhow!("{}", e))?;
        } else {
            tracing::warn!(
                path = %spec_path.to_path_string(),
                "spec command already registered; skipping built-in spec command"
            );
        }

        // Auto-register built-in `completion` command (always-on, opt-out via without_completion()).
        // Build a temporary clap root to capture in the completion command's execute closure.
        if self.auto_register_completion {
            let app_name_for_completion = self.meta.map(|m| m.name).unwrap_or(self.app_name);
            let temp_clap_root = crate::app::clap_adapter::build_clap_root(
                self.meta.as_ref(),
                &self.command_registry,
                self.app_name,
                self.app_version,
                self.app_git_sha_short,
                &self.global_flags,
            );
            let clap_root_arc = Arc::new(temp_clap_root);
            let completion_cmd = crate::command_surface::command::create_completion_command(
                app_name_for_completion,
                clap_root_arc,
            );
            let path = self
                .builtin_command_namespace
                .push("completion")
                .expect("built-in command id is a valid path segment");
            self.command_registry
                .register_framework_completion_at(&path, completion_cmd)
                .map_err(|e| anyhow::anyhow!("{}", e))?;
        }

        // Auto-register `mcp` group + `mcp serve` when mcp-server feature is enabled.
        // Guard: skip if the user already registered a root-level "mcp" command or
        // explicitly registered mcp/serve.
        #[cfg(feature = "mcp-server")]
        {
            let mcp_is_root_cmd = self.command_registry.get("mcp").is_some();
            let mcp_serve_path = CommandPath::new(&["mcp", "serve"]).unwrap();
            let mcp_serve_exists = self.command_registry.resolve(&mcp_serve_path).is_some();

            if self.auto_register_mcp && !mcp_is_root_cmd && !mcp_serve_exists {
                // Register the mcp group node.
                let _ = self.command_registry.register_group(
                    &CommandPath::root_for("mcp"),
                    crate::mcp::commands::mcp_group_metadata(),
                );

                // Clone registry for the serve closure BEFORE registering mcp/serve
                // (so the tool registry used by serve_mcp doesn't include mcp/serve itself).
                let registry_arc_for_serve = Arc::new(self.command_registry.clone());
                let risk_policy_for_serve = self.risk_policy.clone();
                let export_policy_for_serve = self.mcp_export_policy;
                let app_name_for_serve = self.app_name;
                let gate_for_serve = self.mcp_tool_gate.clone();
                let resource_registry_for_serve = self
                    .mcp_resource_registry
                    .clone()
                    .unwrap_or_else(|| Arc::new(crate::mcp::resources::ResourceRegistry::new()));
                let request_authenticator_for_serve = self.mcp_request_authenticator.clone();

                let serve_cmd = crate::mcp::commands::create_mcp_serve_command_with_deps(
                    registry_arc_for_serve,
                    app_name_for_serve,
                    risk_policy_for_serve,
                    export_policy_for_serve,
                    gate_for_serve,
                    resource_registry_for_serve,
                    request_authenticator_for_serve,
                );
                self.command_registry
                    .register_at(&mcp_serve_path, serve_cmd)
                    .expect("mcp serve auto-registration");
            }
        }

        // Auto-register `mcp install` and `mcp list` when mcp-install enabled.
        #[cfg(feature = "mcp-install")]
        {
            if self.command_registry.get("mcp").is_none() {
                let _ = self.command_registry.register_group(
                    &CommandPath::root_for("mcp"),
                    crate::mcp::commands::mcp_group_metadata(),
                );
            }

            let app_name_for_install = self.app_name;
            let install_path = CommandPath::new(&["mcp", "install"]).unwrap();
            if self.command_registry.resolve(&install_path).is_none() {
                let install_cmd =
                    crate::mcp::commands::create_mcp_install_command(app_name_for_install);
                self.command_registry
                    .register_at(&install_path, install_cmd)
                    .expect("mcp install auto-registration");
            }

            let list_path = CommandPath::new(&["mcp", "list"]).unwrap();
            if self.command_registry.resolve(&list_path).is_none() {
                let list_cmd = crate::mcp::commands::create_mcp_list_command();
                self.command_registry
                    .register_at(&list_path, list_cmd)
                    .expect("mcp list auto-registration");
            }
        }

        self.environment_variables
            .register_commands(&self.command_registry)?;

        let mut clap_root = crate::app::clap_adapter::build_clap_root(
            self.meta.as_ref(),
            &self.command_registry,
            self.app_name,
            self.app_version,
            self.app_git_sha_short,
            &self.global_flags,
        );
        if !self.environment_variables.is_empty() {
            clap_root = clap_root.after_help(self.environment_variables.render_help());
        }

        let registry_arc = Arc::new(self.command_registry);

        #[cfg(feature = "config")]
        let (config_handle, config_value_erased) = match config_resolved {
            Some((handle, erased)) => (Some(handle), Some(erased)),
            None => (None, None),
        };

        // `AppContext::opt_config_manifest` sees the *published* manifest, so
        // dispatch, the config commands and the policy server all read the
        // same single document the telemetry section was merged into.
        #[cfg(all(feature = "config", feature = "telemetry"))]
        let config_manifest_for_ctx = Some(published_manifest.clone());
        #[cfg(all(feature = "config", not(feature = "telemetry")))]
        let config_manifest_for_ctx = self.config_manifest.clone();

        Ok(App {
            command_registry: registry_arc,
            ailoop_client,
            plugin_registry_manager,
            ctx,
            meta: self.meta,
            app_name: self.app_name,
            app_version: self.app_version,
            app_git_sha_short: self.app_git_sha_short,
            clap_root,
            global_flags: self.global_flags,
            environment_variables: self.environment_variables,
            stdout_capture: None,
            suggest_corrections: self.suggest_corrections,
            #[cfg(feature = "auth")]
            token_provider: self.token_provider,
            telemetry_config: self.telemetry_config,
            deployment: self.deployment,
            active_telemetry: None,
            #[cfg(feature = "telemetry")]
            telemetry_policy,
            #[cfg(feature = "telemetry")]
            published_manifest,
            #[cfg(feature = "telemetry")]
            telemetry_identity: self.telemetry_identity,
            #[cfg(feature = "telemetry")]
            telemetry_inputs,
            #[cfg(feature = "telemetry")]
            telemetry_store,
            #[cfg(feature = "config")]
            config_handle,
            #[cfg(feature = "config")]
            config_value_erased,
            #[cfg(feature = "config")]
            config_manifest: config_manifest_for_ctx,
            #[cfg(feature = "config-managed")]
            policy_client: self.policy_client,
        })
    }

    /// Like [`Self::build`], but also returns the resolved typed configuration
    /// value registered via [`Self::with_config::<T>`][Self::with_config].
    ///
    /// Fails if `with_config::<T>()` was never called, or was called with a
    /// different type than `T` names here — both surface as a plain error
    /// (there is no dedicated [`crate::config::ConfigError`] variant for a
    /// builder-usage mistake; resolution itself already ran inside `build()`
    /// and any *storage* failure is a `ConfigError` surfaced from there).
    ///
    /// When **both** [`Self::with_config_manifest`] and (under
    /// `config-managed`) [`Self::with_policy_client`] were also called, the
    /// returned value additionally folds in the policy client's **cached**
    /// policy (spec 021, ADR 0072's "enforced beats everything") — a
    /// synchronous, network-free read (see
    /// `crate::config::managed::fold_cached_policy_into_value`); this is what
    /// makes the manifest/resolver/policy-client machinery actually reach an
    /// application's real typed config value, rather than only test code and
    /// hand-written glue ever seeing a policy-aware value. When either is
    /// absent — the overwhelming majority of `with_config::<T>()` callers,
    /// who never touch spec 021 at all — this returns exactly what it always
    /// has: `store.current()` verbatim, with no resolver/manifest involvement
    /// whatsoever.
    ///
    /// This deliberately never fetches over the network: `build()`/
    /// `build_with_config()` are synchronous, and blocking on a runtime here
    /// (or making this function `async`) would be the wrong fix — see
    /// [`crate::config::managed::refresh_managed_config`], the explicit
    /// **async** step an application calls itself (at startup and/or on a
    /// refresh interval) to actually reach the network and push a fresh
    /// policy into a *running* store.
    #[cfg(feature = "config")]
    pub fn build_with_config<C, T>(self, ctx: C) -> Result<(App<C>, T)>
    where
        C: AppContext + 'static,
        T: crate::config::VersionedConfig,
    {
        #[cfg(feature = "config-managed")]
        let managed = self.config_manifest.clone().zip(self.policy_client.clone());

        let app = self.build(ctx)?;
        let store = app.config_store::<T>().ok_or_else(|| {
            anyhow::anyhow!(
                "build_with_config::<{}>() requires a matching with_config::<{}>() call",
                std::any::type_name::<T>(),
                std::any::type_name::<T>()
            )
        })?;

        #[cfg(feature = "config-managed")]
        if let Some((manifest, policy_client)) = managed {
            let value = crate::config::managed::fold_cached_policy_into_value(
                &store,
                &manifest,
                &policy_client,
            )
            .map_err(|e| anyhow::anyhow!("{e}"))?;
            return Ok((app, value));
        }

        let value = (*store.current()).clone();
        Ok((app, value))
    }
}

impl Default for AppBuilder {
    fn default() -> Self {
        Self::new()
    }
}

pub struct App<C: AppContext> {
    command_registry: Arc<CommandRegistry>,
    ailoop_client: Option<AiloopClient>,
    plugin_registry_manager: Option<PluginRegistryManager>,
    ctx: C,
    meta: Option<AppMeta>,
    app_name: &'static str,
    app_version: &'static str,
    app_git_sha_short: Option<&'static str>,
    clap_root: clap::Command,
    global_flags: Vec<ArgSpec>,
    environment_variables: EnvironmentVariableRegistry,
    /// When set, framework-level stdout (`framework_println`: version strings,
    /// help, completion scripts, the command surface, etc.) is captured into this
    /// buffer instead of being written to fd 1. Used by testkit and by embedders
    /// (API hosts) that need to intercept output; `None` writes to real stdout.
    pub stdout_capture: Option<Arc<Mutex<Vec<u8>>>>,
    suggest_corrections: bool,
    #[cfg(feature = "auth")]
    token_provider: Option<Arc<dyn crate::auth::TokenProvider>>,
    #[allow(dead_code)]
    telemetry_config: Option<crate::telemetry::TelemetryConfig>,
    /// The application's deployment shape (spec 025); see
    /// [`AppBuilder::with_deployment`].
    deployment: crate::telemetry::Deployment,
    #[allow(dead_code)]
    pub(crate) active_telemetry: Option<Arc<dyn crate::telemetry::Telemetry + Send + Sync>>,
    /// The one telemetry resolution this process makes (spec 025), settled by
    /// [`AppBuilder::build`] from the deployment shape, the author's
    /// [`TelemetryDefaults`](crate::telemetry::TelemetryDefaults), the
    /// environment and the kill switches.
    ///
    /// Shared rather than copied: the export boundary, the `telemetry`
    /// command group and the `cli.command` probe's span attributes
    /// (`cli.install.id`, `session.id`, `cli.telemetry.level`) all read this
    /// same `Arc`, so no two of them can disagree about what was consented
    /// to.
    ///
    /// Settled at build time, with one exception, and it is deprecated:
    /// [`AppBuilder::with_telemetry`] replaces this whole resolution at
    /// startup with one derived from its `TelemetryConfig`
    /// (see `init_telemetry`). That path predates spec 025 and honours
    /// none of it — no kill switches, no deployment shape, no author probes.
    /// It is scheduled for removal in v0.8.0; once it is gone this field is
    /// written exactly once, by [`AppBuilder::build`].
    ///
    /// The stored consent file is not folded in here — that happens in the
    /// startup sequence, which opens the framework-owned telemetry store and
    /// re-resolves from it. Until that runs, `store_available` is false and
    /// attribution reads `anonymous`, which is the honest answer for a policy
    /// that has not yet read anybody's choice.
    #[cfg(feature = "telemetry")]
    telemetry_policy: Arc<crate::telemetry::TelemetryPolicy>,
    /// The one published manifest (spec 025): the application's own config
    /// manifest with the framework's generated `telemetry` section merged in,
    /// or a telemetry-only manifest when the application publishes none.
    ///
    /// Always present, never optional. An administrator can describe the
    /// telemetry tree of *any* app built on the framework, including one that
    /// has no configuration of its own — which is precisely the app whose
    /// users have no other way to find out what it sends.
    #[cfg(feature = "telemetry")]
    published_manifest: Arc<crate::config::manifest::ConfigManifest>,
    /// The author's identity resolver, if any; see
    /// [`AppBuilder::with_telemetry_identity`].
    #[cfg(feature = "telemetry")]
    telemetry_identity: Option<crate::telemetry::IdentityResolver>,
    /// The build-time telemetry inputs, kept so the startup sequence can
    /// layer the stored consent and the environment on top of them and
    /// resolve once. `telemetry_policy` above is what those inputs resolve
    /// to *before* either is read.
    #[cfg(feature = "telemetry")]
    telemetry_inputs: crate::telemetry::TelemetryInputs,
    /// Where the framework-owned settings file lives. The same value the
    /// `telemetry` command group was registered with, so the group and
    /// startup can never disagree about which file holds consent.
    #[cfg(feature = "telemetry")]
    telemetry_store: crate::telemetry::TelemetryStoreLocation,
    #[cfg(feature = "config")]
    config_handle: Option<Arc<dyn crate::config::ConfigHandle>>,
    #[cfg(feature = "config")]
    config_manifest: Option<Arc<crate::config::manifest::ConfigManifest>>,
    #[cfg(feature = "config-managed")]
    policy_client: Option<Arc<crate::config::managed::PolicyClient>>,
    #[cfg(feature = "config")]
    config_value_erased: Option<Arc<dyn std::any::Any + Send + Sync>>,
}

/// Record a probe's attributes onto an open `cli.command` span.
///
/// `tracing` fixes a callsite's fieldset at compile time (see the limitation
/// documented on [`SpanHandle::set_attr`](crate::telemetry::handle::SpanHandle::set_attr)):
/// a field this span's `info_span!` call did not pre-declare as
/// `tracing::field::Empty` silently drops whatever is recorded for it here.
/// That silence is intentional, not a bug to guard against — it is exactly
/// what lets `command_span_attrs`'s optional `cli.install.id` skip recording
/// for an anonymous install without this function needing to know why.
#[cfg(feature = "telemetry")]
fn record_command_span_attrs(span: &tracing::Span, attrs: &[crate::telemetry::handle::KeyValue]) {
    for kv in attrs {
        span.record(kv.key.as_str(), kv.value.as_str().as_ref());
    }
}

impl<C: AppContext> App<C> {
    #[doc(hidden)]
    pub fn should_show_help(args: &[String]) -> bool {
        args.len() < 2 || args.get(1).is_some_and(|s| s == "--help" || s == "-h")
    }

    pub async fn run(&mut self) -> Result<()> {
        let args: Vec<String> = std::env::args().collect();
        match self.run_with_args(args).await {
            Ok(()) => Ok(()),
            Err(e) => {
                if e.downcast_ref::<UsageError>().is_some() {
                    // Diagnostic already printed by DiagnosticReporter; exit 2 per R5.
                    std::process::exit(2);
                }
                Err(e)
            }
        }
    }

    pub fn rebuild_clap_root(&mut self) {
        self.clap_root = crate::app::clap_adapter::build_clap_root(
            self.meta.as_ref(),
            &self.command_registry,
            self.app_name,
            self.app_version,
            self.app_git_sha_short,
            &self.global_flags,
        );
        if !self.environment_variables.is_empty() {
            self.clap_root = self
                .clap_root
                .clone()
                .after_help(self.environment_variables.render_help());
        }
    }

    /// Returns true if any root-level command or group has a category.
    fn has_categories(&self) -> bool {
        let command_has_categories = self
            .command_registry
            .commands()
            .any(|cmd| cmd.category().is_some());
        let group_has_categories = self
            .command_registry
            .groups()
            .any(|(path, metadata)| !path.contains('/') && metadata.category.is_some());

        command_has_categories || group_has_categories
    }

    pub async fn run_with_args(&mut self, args: Vec<String>) -> Result<()> {
        let _telemetry_guard = self.init_telemetry();
        use crate::app::clap_adapter::parse_with_clap;
        use crate::app::diagnostic_reporter::DiagnosticReporter;
        use crate::parser::diagnostic::{Diagnostic, DiagnosticCategory};
        use crate::parser::error_codes::E_NESTED_COMMAND_NOT_FOUND;
        use crate::parser::outcome::ParseOutcome;

        // Route root-level help through HelpRenderer when any command carries a category.
        if App::<C>::should_show_help(&args) && self.has_categories() {
            self.framework_println(&self.render_help());
            return Ok(());
        }

        #[cfg(not(feature = "chat"))]
        let second_arg = args.get(1).cloned();

        match parse_with_clap(
            &self.clap_root,
            &self.command_registry,
            args,
            &self.global_flags,
            self.suggest_corrections,
        ) {
            ParseOutcome::Parsed {
                command_path,
                args,
                global_args,
            } => {
                let cmd_id = command_path.leaf().unwrap_or("").to_string();
                if cmd_id == "version" && self.command_registry.get("version").is_none() {
                    // Built-in `version` bypasses `execute_command_direct` (it has no
                    // `Command`/`execute` closure to dispatch through — see spec 020
                    // item 6), so it must instrument itself here to stay symmetric with
                    // every other command's `cli.command` span and metrics.
                    #[cfg(feature = "telemetry")]
                    let span = tracing::info_span!(
                        "cli.command",
                        "cli.command.path" = "version",
                        "cli.invocation.surface" = InvocationSurface::Cli.as_str(),
                        "cli.command.arg_count" = 0,
                        "cli.command.arg_names" = "",
                        "cli.probe" = tracing::field::Empty,
                        "cli.install.id" = tracing::field::Empty,
                        "session.id" = tracing::field::Empty,
                        "cli.telemetry.level" = tracing::field::Empty,
                        "command" = tracing::field::Empty,
                        "surface" = tracing::field::Empty,
                        "status" = tracing::field::Empty,
                    )
                    .entered();

                    #[cfg(feature = "telemetry")]
                    let started = std::time::Instant::now();

                    if self.app_name == "unknown" {
                        tracing::warn!("version called but with_version() was not configured");
                    }
                    self.framework_println(&self.version_string());

                    #[cfg(feature = "telemetry")]
                    {
                        let probe_outcome = crate::telemetry::CommandOutcome {
                            command: Some("version".to_string()),
                            surface: crate::telemetry::Surface::Cli,
                            status: crate::telemetry::CommandStatus::Ok,
                            duration_ms: started.elapsed().as_secs_f64() * 1000.0,
                        };
                        record_command_span_attrs(
                            &span,
                            &crate::telemetry::command_span_attrs(
                                &self.telemetry_policy,
                                &probe_outcome,
                            ),
                        );
                        if let Some(telemetry) = self.active_telemetry.as_ref() {
                            let attrs = crate::telemetry::command_metric_labels(&probe_outcome);
                            telemetry.counter("cli.command.invocations").add(1, &attrs);
                            telemetry
                                .histogram("cli.command.duration_ms")
                                .record(probe_outcome.duration_ms, &attrs);
                        }
                    }

                    return Ok(());
                }

                if command_path.0.len() > 1 {
                    // Multi-segment path: use resolve() for dispatch.
                    match self.command_registry.resolve(&command_path) {
                        Some(cmd) => {
                            let diags = crate::app::dispatch::validate_typed_args(cmd, &args);
                            if !diags.is_empty() {
                                DiagnosticReporter::report_all(&diags);
                                return Err(anyhow::Error::new(UsageError(
                                    "validation failed".to_string(),
                                )));
                            }
                            let cmd_clone = cmd.clone();
                            self.execute_command_direct(
                                cmd_clone,
                                args,
                                global_args,
                                InvocationSurface::Cli,
                            )
                            .await
                        }
                        None => {
                            let msg = format!(
                                "nested command '{}' not found",
                                command_path.to_path_string()
                            );
                            DiagnosticReporter::report(&Diagnostic {
                                code: E_NESTED_COMMAND_NOT_FOUND,
                                category: DiagnosticCategory::Parse,
                                message: msg.clone(),
                                suggestion: Some(
                                    "Use --help to see available commands".to_string(),
                                ),
                                span: None,
                            });
                            Err(anyhow::Error::new(UsageError(msg)))
                        }
                    }
                } else {
                    self.execute_command_with_globals(&cmd_id, args, global_args)
                        .await
                }
            }
            ParseOutcome::HelpShown(text) => {
                self.framework_println(text.trim_end());
                Ok(())
            }
            ParseOutcome::VersionShown(text) => {
                self.framework_println(text.trim_end());
                Ok(())
            }
            ParseOutcome::ParseError(d) => {
                #[cfg(not(feature = "chat"))]
                {
                    // Deterministic error when `chat` is invoked without the `chat` feature.
                    if d.code == crate::parser::error_codes::E_UNKNOWN_COMMAND
                        && second_arg.as_deref() == Some("chat")
                    {
                        return Err(anyhow::anyhow!(
                            "CHAT_FEATURE_DISABLED: `chat` requires building with `--features chat`"
                        ));
                    }
                }
                DiagnosticReporter::report(&d);
                Err(anyhow::Error::new(UsageError(d.message.clone())))
            }
        }
    }

    /// Write a line of framework-level output. Routes through the testkit capture buffer
    /// when active; otherwise writes to real stdout.
    fn framework_println(&self, s: &str) {
        use std::io::Write;

        if let Some(ref buf) = self.stdout_capture {
            let mut lock = buf.lock().unwrap_or_else(|e| e.into_inner());
            lock.extend_from_slice(s.as_bytes());
            lock.push(b'\n');
            return;
        }

        let mut stdout = std::io::stdout();
        let _ = writeln!(stdout, "{}", s);
    }

    pub fn show_help(&self) {
        HelpRenderer::new(self.meta.as_ref(), self.command_registry.as_ref())
            .with_version_string(self.version_string())
            .with_global_flags(&self.global_flags)
            .with_environment_variables(&self.environment_variables)
            .print();
    }

    pub fn render_help(&self) -> String {
        HelpRenderer::new(self.meta.as_ref(), self.command_registry.as_ref())
            .with_version_string(self.version_string())
            .with_global_flags(&self.global_flags)
            .with_environment_variables(&self.environment_variables)
            .render()
    }

    pub fn version_string(&self) -> String {
        let app_name = self.meta.map(|m| m.name).unwrap_or(self.app_name);
        let app_version = self.meta.map(|m| m.version).unwrap_or(self.app_version);
        crate::app::version::format_display_version(app_name, app_version, self.app_git_sha_short)
    }

    pub fn emit_completion(
        &self,
        shell: Shell,
        out: &mut dyn std::io::Write,
    ) -> anyhow::Result<()> {
        let app_name = self.meta.as_ref().map(|m| m.name).unwrap_or(self.app_name);
        let model = build_completion_model(self.command_registry.as_ref());
        emit_completion_script(app_name, shell, &model, out)
    }

    /// Execute a root-level command by ID with a typed argument map.
    pub async fn execute_command(
        &mut self,
        command_id: &str,
        args: HashMap<String, ArgValue>,
    ) -> Result<()> {
        let command = self
            .command_registry
            .get(command_id)
            .ok_or_else(|| anyhow::anyhow!("Command '{}' not found", command_id))?
            .clone();
        self.execute_command_direct(command, args, HashMap::new(), InvocationSurface::Cli)
            .await
    }

    /// Execute a root-level command by ID with typed argument map and global args.
    async fn execute_command_with_globals(
        &mut self,
        command_id: &str,
        args: HashMap<String, ArgValue>,
        global_args: HashMap<String, ArgValue>,
    ) -> Result<()> {
        let command = self
            .command_registry
            .get(command_id)
            .ok_or_else(|| anyhow::anyhow!("Command '{}' not found", command_id))?
            .clone();
        let diags = crate::app::dispatch::validate_typed_args(&command, &args);
        if !diags.is_empty() {
            use crate::app::diagnostic_reporter::DiagnosticReporter;
            DiagnosticReporter::report_all(&diags);
            return Err(anyhow::Error::new(UsageError(
                "validation failed".to_string(),
            )));
        }
        self.execute_command_direct(command, args, global_args, InvocationSurface::Cli)
            .await
    }

    /// Execute an already-resolved `Command` with a typed argument map and global args.
    /// Shared by both single-segment and multi-segment dispatch paths in `run_with_args`.
    async fn execute_command_direct(
        &mut self,
        command: Command,
        args: HashMap<String, ArgValue>,
        global_args: HashMap<String, ArgValue>,
        surface: InvocationSurface,
    ) -> Result<()> {
        let env = crate::app::dispatch::DispatchEnv {
            command_registry: self.command_registry.as_ref(),
            ailoop_client: &self.ailoop_client,
            global_args: &global_args,
            stdout_capture: self.stdout_capture.clone(),
            telemetry: self.active_telemetry.clone(),
            #[cfg(feature = "telemetry")]
            probe_registry: &self.telemetry_policy.registry,
            surface,
            #[cfg(feature = "auth")]
            token_provider: self.token_provider.clone(),
            #[cfg(feature = "config")]
            config_handle: self.config_handle.clone(),
            #[cfg(feature = "config")]
            config_manifest: self.config_manifest.clone(),
            #[cfg(feature = "config-managed")]
            policy_client: self.policy_client.clone(),
        };

        #[cfg(feature = "telemetry")]
        let cmd_id = command.id.as_ref().to_string();

        #[cfg(feature = "telemetry")]
        let span = {
            let arg_names: String = args.keys().cloned().collect::<Vec<_>>().join(",");
            tracing::info_span!(
                "cli.command",
                "cli.command.path" = cmd_id.as_str(),
                "cli.invocation.surface" = surface.as_str(),
                "cli.command.arg_count" = args.len(),
                "cli.command.arg_names" = arg_names.as_str(),
                "cli.probe" = tracing::field::Empty,
                "cli.install.id" = tracing::field::Empty,
                "session.id" = tracing::field::Empty,
                "cli.telemetry.level" = tracing::field::Empty,
                "command" = tracing::field::Empty,
                "surface" = tracing::field::Empty,
                "status" = tracing::field::Empty,
            )
            .entered()
        };

        #[cfg(feature = "telemetry")]
        let started = std::time::Instant::now();

        let mut ctx_wrapper = crate::app::dispatch::CliAppContextWrapper::new(&mut self.ctx, env);

        let outcome = (command.execute)(&mut ctx_wrapper, args).await;

        // Auto per-command metrics (spec 017 "Command span and metrics shape").
        // Emitted on the error path too — an error-rate metric that only counts
        // successes is worse than no metric at all.
        #[cfg(feature = "telemetry")]
        {
            let probe_outcome = crate::telemetry::CommandOutcome {
                command: Some(cmd_id),
                surface: surface.into(),
                status: if outcome.is_ok() {
                    crate::telemetry::CommandStatus::Ok
                } else {
                    crate::telemetry::CommandStatus::Error
                },
                duration_ms: started.elapsed().as_secs_f64() * 1000.0,
            };
            record_command_span_attrs(
                &span,
                &crate::telemetry::command_span_attrs(&self.telemetry_policy, &probe_outcome),
            );
            if let Some(telemetry) = self.active_telemetry.as_ref() {
                let attrs = crate::telemetry::command_metric_labels(&probe_outcome);
                telemetry.counter("cli.command.invocations").add(1, &attrs);
                telemetry
                    .histogram("cli.command.duration_ms")
                    .record(probe_outcome.duration_ms, &attrs);
            }
        }

        outcome
    }

    /// Initialise the export pipeline for a CLI run.
    ///
    /// Uses `init_batch`, not `init_simple`: this is called from the `async`
    /// [`run_with_args`](Self::run_with_args), and `SimpleSpanProcessor` exports
    /// inline through `reqwest::blocking`, which panics with *"Cannot drop a
    /// runtime in a context where blocking is not allowed"* on the first span
    /// close inside a Tokio worker. The guard flushes on drop, so a short-lived
    /// CLI process still delivers its spans.
    #[cfg(feature = "telemetry")]
    fn init_telemetry(&mut self) -> crate::telemetry::TelemetryGuard {
        if let Some(ref cfg) = self.telemetry_config {
            let svc = self.meta.as_ref().map(|m| m.name).unwrap_or(self.app_name);
            let ver = self
                .meta
                .as_ref()
                .map(|m| m.version)
                .unwrap_or(self.app_version);

            // Overwrite the resolution `build` made. This is a regression on
            // paper — the replacement honours no kill switch, no deployment
            // shape and no author-registered probe — and it is deliberate:
            // an app still on the deprecated `with_telemetry` shim has always
            // exported unconditionally to its configured endpoint, and
            // quietly turning that off underneath it would be a behaviour
            // change smuggled in under a compatibility shim. The shim is
            // deprecated in v0.6.0 and removed in v0.8.0; the resolution
            // `build` made is what survives it.
            self.telemetry_policy = Arc::new(crate::telemetry::resolve_policy(
                crate::telemetry::TelemetryInputs {
                    app: svc.to_string(),
                    deployment: crate::telemetry::Deployment::Service,
                    endpoint: cfg.endpoint.clone(),
                    session_id: uuid::Uuid::new_v4().to_string(),
                    sample_ratio: cfg.sample_ratio,
                    registry: crate::telemetry::ProbeRegistry::with_builtins(),
                    ..Default::default()
                },
            ));

            if let Some((handle, guard)) = crate::telemetry::init::init_batch(cfg, svc, ver) {
                self.active_telemetry = Some(handle);
                return guard;
            }
            // The shim was configured but produced nothing usable. Fall
            // through to the spec 025 sequence rather than returning a dead
            // guard: it still has to open the store, honour the kill
            // switches and show the notice.
        }

        // Spec 025 startup. `run_startup` walks all ten steps in order --
        // kill switches, the store, the environment, one resolution, the
        // freeze, the providers and export boundary, the subscriber, the
        // notice, the panic hook, dispatch -- and returns what each produced.
        let result = crate::telemetry::run_startup(self.startup_inputs());
        self.telemetry_policy = result.policy.clone();
        self.active_telemetry = result.handle.clone();
        if let Some(notice) = result.notice.as_deref() {
            // Straight to stderr, not through `tracing`: the notice is a
            // message to the person at the terminal, not a log line, and it
            // has to appear whether or not the app installed a subscriber.
            eprintln!("{notice}");
        }
        result.guard.unwrap_or_else(|| {
            // Nothing was built, because the policy does not export. The
            // guard still exists because `run_with_args` holds one
            // unconditionally; flushing it is a no-op.
            let provider = opentelemetry_sdk::trace::SdkTracerProvider::builder().build();
            crate::telemetry::TelemetryGuard::new(provider, None)
        })
    }

    /// Everything [`run_startup`](crate::telemetry::run_startup) reads,
    /// gathered in one place.
    ///
    /// The ambient process state startup depends on -- the environment, and
    /// whether stderr is a terminal -- is captured *here* rather than inside
    /// the sequence. That is what lets a test substitute all of it and
    /// assert the startup order by running startup, instead of re-reading
    /// the constant startup is supposed to follow.
    #[cfg(feature = "telemetry")]
    #[doc(hidden)]
    pub fn startup_inputs(&self) -> crate::telemetry::StartupInputs {
        use std::io::IsTerminal;
        crate::telemetry::StartupInputs {
            base: self.telemetry_inputs.clone(),
            store: self.telemetry_store.clone(),
            manifest: self.published_manifest.clone(),
            service: crate::telemetry::ServiceIdentity {
                name: self
                    .meta
                    .as_ref()
                    .map(|m| m.name)
                    .unwrap_or(self.app_name)
                    .to_string(),
                version: self
                    .meta
                    .as_ref()
                    .map(|m| m.version)
                    .unwrap_or(self.app_version)
                    .to_string(),
            },
            // `run_with_args` is the command-line surface by construction.
            // Chat, MCP and the API server reach telemetry through their own
            // entry points and pass their own surface.
            surface: crate::telemetry::Surface::Cli,
            stderr_is_tty: std::io::stderr().is_terminal(),
            env: std::env::vars().collect(),
        }
    }

    #[cfg(not(feature = "telemetry"))]
    fn init_telemetry(&mut self) -> crate::telemetry::TelemetryGuard {
        crate::telemetry::TelemetryGuard
    }

    /// Return a reference to the command registry.
    pub fn command_registry(&self) -> &CommandRegistry {
        self.command_registry.as_ref()
    }

    /// Return the global flags registered on this app.
    pub fn global_flags(&self) -> &[ArgSpec] {
        &self.global_flags
    }

    /// Return all application and command environment variable declarations.
    pub fn environment_variables(&self) -> &EnvironmentVariableRegistry {
        &self.environment_variables
    }

    /// The deployment shape configured via [`AppBuilder::with_deployment`].
    pub fn deployment(&self) -> &crate::telemetry::Deployment {
        &self.deployment
    }

    pub fn ailoop_client(&self) -> Option<&AiloopClient> {
        self.ailoop_client.as_ref()
    }

    pub fn has_plugins(&self) -> bool {
        self.plugin_registry_manager.is_some()
    }

    /// Recover the typed `ConfigStore<T>` registered via
    /// `AppBuilder::with_config::<T>()`, for applications that need
    /// reload/subscribe access (long-running apps — see spec 016 user
    /// stories 16-17) rather than just the one-shot resolved value returned
    /// by [`AppBuilder::build_with_config`].
    ///
    /// Returns `None` if `with_config::<T>()` was never called, or was
    /// called with a different `T` than requested here. The returned `Arc`
    /// is the *same* store instance backing `AppContext::opt_config_handle`,
    /// so calling `reload()` through either one updates both and notifies
    /// subscribers registered through either one.
    #[cfg(feature = "config")]
    pub fn config_store<T>(&self) -> Option<Arc<crate::config::ConfigStore<T>>>
    where
        T: crate::config::VersionedConfig,
    {
        self.config_value_erased
            .clone()?
            .downcast::<crate::config::ConfigStore<T>>()
            .ok()
    }

    /// The telemetry policy this process resolved at build time (spec 025).
    #[cfg(feature = "telemetry")]
    pub fn telemetry_policy(&self) -> &crate::telemetry::TelemetryPolicy {
        &self.telemetry_policy
    }

    /// The published config manifest — the app's own fields plus the
    /// framework's generated `telemetry` section.
    ///
    /// Total, unlike the app-declared manifest it is built from: an app that
    /// declared none still publishes the telemetry tree. This is the document
    /// `AppContext::opt_config_manifest` hands to the `config` commands and to
    /// the managed-configuration server, so what an administrator can inspect
    /// and what the framework will actually honour are the same list.
    #[cfg(feature = "telemetry")]
    pub fn config_manifest(&self) -> &crate::config::manifest::ConfigManifest {
        &self.published_manifest
    }

    /// The author's identity resolver, if one was registered through
    /// [`AppBuilder::with_telemetry_identity`].
    #[cfg(feature = "telemetry")]
    pub fn telemetry_identity_resolver(&self) -> Option<&crate::telemetry::IdentityResolver> {
        self.telemetry_identity.as_ref()
    }
}

/// The context a test app runs with: no registry, no output capture, no
/// token provider — every [`AppContext`] method left at its default.
///
/// Exists so a test can exercise the *builder* without also having to stand
/// up a host application. Hidden from the public docs because it is a test
/// seam, not API surface an app author should reach for.
#[doc(hidden)]
#[derive(Debug, Default)]
pub struct TestContext;

impl AppContext for TestContext {}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Shell {
    Bash,
    Zsh,
    Fish,
    PowerShell,
}

/// The words that may legally follow one command path.
///
/// One of these is produced per reachable node of the command tree — the root
/// (`""`), every group, and every leaf command — so a shell completion script
/// can offer the right candidates at the position the cursor is actually on
/// instead of one flat top-level list.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub(crate) struct CompletionNode {
    /// Direct child command/group segments registered under this path.
    pub(crate) subcommands: BTreeSet<String>,
    /// Flag tokens accepted here (`--long`, and `-s` where a short is declared).
    pub(crate) flags: BTreeSet<String>,
}

/// The whole completion surface, keyed by space-joined command path
/// (`""` for the root, `"mcp"`, `"mcp serve"`, …).
#[derive(Debug, Default, Clone)]
pub(crate) struct CompletionModel {
    pub(crate) nodes: BTreeMap<String, CompletionNode>,
}

impl CompletionModel {
    /// Candidates offered when the cursor is on the first word.
    pub(crate) fn top_level_commands(&self) -> BTreeSet<String> {
        self.nodes
            .get("")
            .map(|n| n.subcommands.clone())
            .unwrap_or_default()
    }
}

/// Build the completion surface from the command registry.
///
/// Hidden commands and hidden groups contribute nothing of their own.
/// Hiding is per node, not per subtree — exactly as `build_clap_root` treats
/// it: a *visible* leaf under a hidden group is still routable, so its ancestor
/// segments stay completable (the long-standing contract asserted by
/// `completion_includes_root_segment_from_visible_leaf_even_when_group_hidden`).
///
/// `--help` is added at every level because clap supplies it on every node it
/// builds.
pub(crate) fn build_completion_model(registry: &CommandRegistry) -> CompletionModel {
    let mut model = CompletionModel::default();
    model.nodes.entry(String::new()).or_default();

    for (path_str, meta) in registry.groups() {
        if meta.hidden {
            continue;
        }
        insert_completion_path(&mut model, path_str);
    }

    for (path_str, cmd) in registry.all_tree_commands() {
        if cmd.spec.hidden {
            continue;
        }
        insert_completion_path(&mut model, path_str);

        let node = model
            .nodes
            .entry(completion_key(path_str))
            .or_insert_with(CompletionNode::default);
        for arg in &cmd.spec.args {
            match arg.kind {
                crate::spec::arg_spec::ArgKind::Flag | crate::spec::arg_spec::ArgKind::Option => {
                    // Same derivation clap itself uses (`build_clap_arg`):
                    // the `long` override when present, else the arg name.
                    node.flags
                        .insert(format!("--{}", arg.long.unwrap_or(arg.name)));
                    if let Some(short) = arg.short {
                        node.flags.insert(format!("-{}", short));
                    }
                }
                crate::spec::arg_spec::ArgKind::Positional => {}
            }
        }
    }

    for node in model.nodes.values_mut() {
        node.flags.insert("--help".to_string());
    }

    model
}

/// Space-joined completion key for a registry path string (`"mcp/serve"` → `"mcp serve"`).
fn completion_key(path_str: &str) -> String {
    path_str
        .split('/')
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

/// Record `path_str` in the model: every segment becomes a subcommand of its
/// parent, and every prefix gets a node of its own.
fn insert_completion_path(model: &mut CompletionModel, path_str: &str) {
    let mut parent = String::new();
    for segment in path_str.split('/').filter(|s| !s.is_empty()) {
        model
            .nodes
            .entry(parent.clone())
            .or_default()
            .subcommands
            .insert(segment.to_string());
        parent = if parent.is_empty() {
            segment.to_string()
        } else {
            format!("{} {}", parent, segment)
        };
    }
    model.nodes.entry(parent).or_default();
}

pub(crate) fn emit_completion_script(
    app_name: &str,
    shell: Shell,
    model: &CompletionModel,
    out: &mut dyn std::io::Write,
) -> anyhow::Result<()> {
    let cmds = model.top_level_commands();
    let cmds = &cmds;
    match shell {
        Shell::Bash => {
            let fn_name = format!("_{}", app_name);
            writeln!(out, "{}() {{", fn_name)?;
            // The word under the cursor — NOT COMP_WORDS[1], which is only ever
            // the first argument and makes every position past it complete
            // against the wrong word.
            writeln!(out, "  local cur=\"${{COMP_WORDS[COMP_CWORD]}}\"")?;
            writeln!(out, "  local path=\"\" i w")?;
            // Rebuild the command path from the non-flag words before the
            // cursor, so candidates are chosen for the level being completed.
            writeln!(out, "  for (( i=1; i < COMP_CWORD; i++ )); do")?;
            writeln!(out, "    w=\"${{COMP_WORDS[i]}}\"")?;
            writeln!(out, "    case \"$w\" in -*) continue ;; esac")?;
            writeln!(
                out,
                "    if [[ -z $path ]]; then path=\"$w\"; else path=\"$path $w\"; fi"
            )?;
            writeln!(out, "  done")?;
            writeln!(out)?;
            writeln!(out, "  local candidates=\"\"")?;
            writeln!(out, "  case \"$path\" in")?;
            for (path, node) in &model.nodes {
                // One deterministic, sorted list per level: subcommands and flags
                // are interchangeable candidates to `compgen -W`.
                let words: BTreeSet<&String> =
                    node.subcommands.iter().chain(node.flags.iter()).collect();
                writeln!(
                    out,
                    "    {}) candidates=\"{}\" ;;",
                    bash_single_quote(path),
                    words
                        .into_iter()
                        .map(|s| s.as_str())
                        .collect::<Vec<_>>()
                        .join(" ")
                )?;
            }
            writeln!(out, "    *) candidates=\"\" ;;")?;
            writeln!(out, "  esac")?;
            writeln!(out)?;
            writeln!(
                out,
                "  COMPREPLY=( $(compgen -W \"$candidates\" -- \"$cur\") )"
            )?;
            writeln!(out, "}}")?;
            // `-o default`: fall back to readline's filename completion when this
            // function produces nothing, rather than suppressing it outright.
            writeln!(out, "complete -o default -F {} {}", fn_name, app_name)?;
        }
        Shell::Zsh => {
            let fn_name = format!("_{}", app_name);
            writeln!(out, "#compdef {}", app_name)?;
            writeln!(out)?;
            writeln!(out, "{}() {{", fn_name)?;
            writeln!(out, "  local -a commands")?;
            writeln!(out, "  commands=(")?;
            for cmd in cmds {
                writeln!(out, "    '{}'", cmd)?;
            }
            writeln!(out, "  )")?;
            writeln!(out, "  _describe 'command' commands")?;
            writeln!(out, "}}")?;
            writeln!(out)?;
            writeln!(out, "compdef {} {}", fn_name, app_name)?;
        }
        Shell::Fish => {
            writeln!(out, "complete -c {} -f", app_name)?;
            for cmd in cmds {
                writeln!(
                    out,
                    "complete -c {} -n '__fish_use_subcommand' -a '{}'",
                    app_name, cmd
                )?;
            }
        }
        Shell::PowerShell => {
            writeln!(
                out,
                "Register-ArgumentCompleter -Native -CommandName {} -ScriptBlock {{",
                app_name
            )?;
            writeln!(
                out,
                "  param($commandName, $wordToComplete, $cursorPosition)"
            )?;
            writeln!(out, "  $candidates = @(")?;
            for cmd in cmds {
                writeln!(out, "    '{}'", cmd)?;
            }
            writeln!(out, "  )")?;
            writeln!(
                out,
                "  $candidates | Where-Object {{ $_ -like \"$wordToComplete*\" }} | ForEach-Object {{"
            )?;
            writeln!(
                out,
                "    [System.Management.Automation.CompletionResult]::new($_, $_, 'ParameterValue', $_)"
            )?;
            writeln!(out, "  }}")?;
            writeln!(out, "}}")?;
        }
    }

    Ok(())
}

/// Wrap `s` as a bash single-quoted word, so it is matched literally when used
/// as a `case` pattern (an empty path becomes `''`, bash's empty-string pattern).
fn bash_single_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', r"'\''"))
}
