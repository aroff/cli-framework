use crate::ailoop::{AiloopClient, AiloopConfig};
use crate::app::context::AppContext;
use crate::app::module::Module;
use crate::app::AppMeta;
use crate::cli_output::HelpRenderer;
use crate::command::{Command, CommandRegistry, TypedArgs};
use crate::plugin::PluginRegistryManager;
use crate::spec::arg_spec::ArgSpec;
use crate::spec::command_tree::{CommandPath, GroupMetadata};
use crate::spec::value::ArgValue;
use anyhow::Result;
use std::collections::{BTreeSet, HashMap};
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
    global_flags: Vec<ArgSpec>,
    #[cfg(feature = "doctor")]
    doctor_checks: Vec<Arc<dyn crate::doctor::check::DoctorCheck>>,
    #[cfg(feature = "mcp-server")]
    mcp_export_policy: crate::mcp::McpToolExportPolicy,
    #[cfg(feature = "mcp-server")]
    mcp_tool_gate: Option<std::sync::Arc<dyn crate::security::ExecutionGate>>,
    #[cfg(feature = "mcp-server")]
    mcp_resource_registry: Option<std::sync::Arc<crate::mcp::resources::ResourceRegistry>>,
    #[cfg(feature = "chat")]
    chat_tool_policy: crate::command::chat::ChatToolPolicy,
    suggest_corrections: bool,
}

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
            global_flags: Vec::new(),
            #[cfg(feature = "doctor")]
            doctor_checks: Vec::new(),
            #[cfg(feature = "mcp-server")]
            mcp_export_policy: crate::mcp::McpToolExportPolicy::default(),
            #[cfg(feature = "mcp-server")]
            mcp_tool_gate: None,
            #[cfg(feature = "mcp-server")]
            mcp_resource_registry: None,
            #[cfg(feature = "chat")]
            chat_tool_policy: crate::command::chat::ChatToolPolicy::default(),
            suggest_corrections: true,
        }
    }

    /// Enable or disable "Did you mean?" suggestions for unknown subcommands
    /// and flags. Default: `true`.
    pub fn suggest_corrections(mut self, enabled: bool) -> Self {
        self.suggest_corrections = enabled;
        self
    }

    /// Disable auto-registration of the built-in `completion` command.
    pub fn without_completion(mut self) -> Self {
        self.auto_register_completion = false;
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

    pub fn build<C: AppContext + 'static>(mut self, ctx: C) -> Result<App<C>> {
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

        // Auto-register built-in `spec` command (always-on, no feature gate)
        if self.command_registry.get("spec").is_none() {
            let spec_cmd = crate::command_surface::command::create_spec_command(
                self.app_name,
                self.app_version,
            );
            self.command_registry.register(spec_cmd);
        } else {
            tracing::warn!("'spec' command already registered; skipping built-in spec command");
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
            let path = CommandPath::root_for("completion");
            self.command_registry
                .register_at(&path, completion_cmd)
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

            if !mcp_is_root_cmd && !mcp_serve_exists {
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

                let serve_cmd = crate::mcp::commands::create_mcp_serve_command_with_deps(
                    registry_arc_for_serve,
                    app_name_for_serve,
                    risk_policy_for_serve,
                    export_policy_for_serve,
                    gate_for_serve,
                    resource_registry_for_serve,
                );
                self.command_registry
                    .register_at(&mcp_serve_path, serve_cmd)
                    .expect("mcp serve auto-registration");
            }
        }

        // Auto-register `mcp install` (alias `register`) and `mcp list` when mcp-install enabled.
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
                let mut register_cmd = install_cmd.clone();
                register_cmd.id = Arc::from("register");
                self.command_registry
                    .register_at(&install_path, install_cmd)
                    .expect("mcp install auto-registration");
                let register_path = CommandPath::new(&["mcp", "register"]).unwrap();
                if self.command_registry.resolve(&register_path).is_none() {
                    self.command_registry
                        .register_at(&register_path, register_cmd)
                        .expect("mcp register auto-registration");
                }
            }

            let list_path = CommandPath::new(&["mcp", "list"]).unwrap();
            if self.command_registry.resolve(&list_path).is_none() {
                let list_cmd = crate::mcp::commands::create_mcp_list_command();
                self.command_registry
                    .register_at(&list_path, list_cmd)
                    .expect("mcp list auto-registration");
            }
        }

        let clap_root = crate::app::clap_adapter::build_clap_root(
            self.meta.as_ref(),
            &self.command_registry,
            self.app_name,
            self.app_version,
            self.app_git_sha_short,
            &self.global_flags,
        );

        let registry_arc = Arc::new(self.command_registry);

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
            stdout_capture: None,
            suggest_corrections: self.suggest_corrections,
        })
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
    /// When set, framework-level stdout (`framework_println`: version strings,
    /// help, completion scripts, the command surface, etc.) is captured into this
    /// buffer instead of being written to fd 1. Used by testkit and by embedders
    /// (API hosts) that need to intercept output; `None` writes to real stdout.
    pub stdout_capture: Option<Arc<Mutex<Vec<u8>>>>,
    suggest_corrections: bool,
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
    }

    /// Returns true if any root-level command has a non-None category.
    fn has_categories(&self) -> bool {
        self.command_registry
            .commands()
            .any(|cmd| cmd.category().is_some())
    }

    pub async fn run_with_args(&mut self, args: Vec<String>) -> Result<()> {
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
                    if self.app_name == "unknown" {
                        tracing::warn!("version called but with_version() was not configured");
                    }
                    self.framework_println(&self.version_string());
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
                            self.execute_command_direct(cmd_clone, args, global_args)
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
            .print();
    }

    pub fn render_help(&self) -> String {
        HelpRenderer::new(self.meta.as_ref(), self.command_registry.as_ref())
            .with_version_string(self.version_string())
            .with_global_flags(&self.global_flags)
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
        let cmds = visible_top_level_commands(self.command_registry.as_ref());
        emit_completion_script(app_name, shell, &cmds, out)
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
        self.execute_command_direct(command, args, HashMap::new())
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
        self.execute_command_direct(command, args, global_args)
            .await
    }

    /// Execute an already-resolved `Command` with a typed argument map and global args.
    /// Shared by both single-segment and multi-segment dispatch paths in `run_with_args`.
    async fn execute_command_direct(
        &mut self,
        command: Command,
        args: HashMap<String, ArgValue>,
        global_args: HashMap<String, ArgValue>,
    ) -> Result<()> {
        let env = crate::app::dispatch::DispatchEnv {
            command_registry: self.command_registry.as_ref(),
            ailoop_client: &self.ailoop_client,
            global_args: &global_args,
            stdout_capture: self.stdout_capture.clone(),
        };
        let mut ctx_wrapper = crate::app::dispatch::CliAppContextWrapper::new(&mut self.ctx, env);

        (command.execute)(&mut ctx_wrapper, args).await?;
        Ok(())
    }

    /// Return a reference to the command registry.
    pub fn command_registry(&self) -> &CommandRegistry {
        self.command_registry.as_ref()
    }

    /// Return the global flags registered on this app.
    pub fn global_flags(&self) -> &[ArgSpec] {
        &self.global_flags
    }

    pub fn ailoop_client(&self) -> Option<&AiloopClient> {
        self.ailoop_client.as_ref()
    }

    pub fn has_plugins(&self) -> bool {
        self.plugin_registry_manager.is_some()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Shell {
    Bash,
    Zsh,
    Fish,
    PowerShell,
}

pub(crate) fn visible_top_level_commands(registry: &CommandRegistry) -> BTreeSet<String> {
    let mut out = BTreeSet::new();

    for (path_str, meta) in registry.groups() {
        if meta.hidden {
            continue;
        }
        if let Some(root) = path_str.split('/').next().filter(|s| !s.is_empty()) {
            out.insert(root.to_string());
        }
    }

    for (path_str, cmd) in registry.all_tree_commands() {
        if cmd.spec.hidden {
            continue;
        }
        if let Some(root) = path_str.split('/').next().filter(|s| !s.is_empty()) {
            out.insert(root.to_string());
        }
    }

    out
}

pub(crate) fn emit_completion_script(
    app_name: &str,
    shell: Shell,
    cmds: &BTreeSet<String>,
    out: &mut dyn std::io::Write,
) -> anyhow::Result<()> {
    match shell {
        Shell::Bash => {
            let fn_name = format!("_{}", app_name);
            writeln!(out, "{}() {{", fn_name)?;
            writeln!(out, "  local cur=\"${{COMP_WORDS[1]}}\"")?;
            writeln!(
                out,
                "  COMPREPLY=( $(compgen -W \"{}\" -- \"$cur\") )",
                join_space(cmds)
            )?;
            writeln!(out, "}}")?;
            writeln!(out, "complete -F {} {}", fn_name, app_name)?;
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

fn join_space(cmds: &BTreeSet<String>) -> String {
    cmds.iter().cloned().collect::<Vec<_>>().join(" ")
}
