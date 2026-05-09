use crate::ailoop::{AiloopClient, AiloopConfig};
use crate::app::context::AppContext;
use crate::app::module::Module;
use crate::app::AppMeta;
use crate::cli_output::HelpRenderer;
use crate::command::{Command, CommandRegistry};
use crate::llm::LlmProvider;
use crate::parser::validator::SpecValidator;
use crate::plugin::PluginRegistryManager;
use crate::spec::command_tree::{CommandPath, GroupMetadata};
use crate::spec::value::ArgValue;
use anyhow::Result;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

#[cfg(feature = "testkit")]
use std::sync::Mutex;

pub struct AppBuilder {
    command_registry: CommandRegistry,
    plugin_registry_manager: Option<PluginRegistryManager>,
    llm_provider: Option<Arc<dyn LlmProvider>>,
    ailoop_config: Option<AiloopConfig>,
    plugin_registry_path: Option<PathBuf>,
    meta: Option<AppMeta>,
    app_name: &'static str,
    app_version: &'static str,
    risk_policy: crate::security::command_risk::CommandRiskPolicy,
    #[cfg(feature = "doctor")]
    doctor_checks: Vec<Arc<dyn crate::doctor::check::DoctorCheck>>,
    #[cfg(feature = "mcp-server")]
    mcp_export_policy: crate::mcp::McpToolExportPolicy,
}

impl AppBuilder {
    pub fn new() -> Self {
        Self {
            command_registry: CommandRegistry::new(),
            plugin_registry_manager: None,
            llm_provider: None,
            ailoop_config: None,
            plugin_registry_path: None,
            meta: None,
            app_name: "unknown",
            app_version: "unknown",
            risk_policy: crate::security::command_risk::CommandRiskPolicy::default(),
            #[cfg(feature = "doctor")]
            doctor_checks: Vec::new(),
            #[cfg(feature = "mcp-server")]
            mcp_export_policy: crate::mcp::McpToolExportPolicy::default(),
        }
    }

    /// Set the MCP export policy used when `--mcp-serve` starts the embedded server.
    /// Default: `McpToolExportPolicy::AllCommands` (backward compatible).
    #[cfg(feature = "mcp-server")]
    pub fn with_mcp_export_policy(mut self, policy: crate::mcp::McpToolExportPolicy) -> Self {
        self.mcp_export_policy = policy;
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

    /// Register a root-level command. Returns `Err` if the command ID is already occupied
    /// or an alias conflicts with an existing registration.
    pub fn register_command(mut self, command: Command) -> Result<Self> {
        #[cfg(feature = "strict-types")]
        if command.spec.is_none() {
            return Err(anyhow::anyhow!(
                "strict-types: command '{}' must have a CommandSpec",
                command.id
            ));
        }

        let path = CommandPath::root_for(command.id);
        self.command_registry
            .register_at(&path, command)
            .map_err(|e| anyhow::anyhow!("{}", e))?;
        Ok(self)
    }

    /// Register a command at an arbitrary `CommandPath`.
    pub fn register_command_at(mut self, path: &CommandPath, command: Command) -> Result<Self> {
        #[cfg(feature = "strict-types")]
        if command.spec.is_none() {
            return Err(anyhow::anyhow!(
                "strict-types: command at path '{}' must have a CommandSpec",
                path.to_path_string()
            ));
        }

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

    pub fn with_llm_provider(mut self, provider: Arc<dyn LlmProvider>) -> Self {
        self.llm_provider = Some(provider);
        self
    }

    pub fn with_llm_from_env(mut self) -> Result<Self> {
        let provider = crate::llm::LlmProviderFactory::from_env()?;
        self.llm_provider = Some(provider);
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
        if self.llm_provider.is_some() && self.ailoop_config.is_none() {
            return Err(anyhow::anyhow!(
                "AILOOP_REQUIRED_FOR_ASK: ask command requires ailoop configuration; \
                 call .with_ailoop_config() or .with_ailoop_channel() before .build()"
            ));
        }

        let ailoop_client = if let Some(config) = self.ailoop_config {
            Some(AiloopClient::with_config(config)?)
        } else {
            None
        };

        let plugin_registry_manager = self.plugin_registry_manager;

        if let Some(ref provider) = self.llm_provider {
            let registry_snapshot = Arc::new(self.command_registry.clone());
            let ask_command = crate::command::create_ask_command(
                provider.clone(),
                registry_snapshot,
                self.risk_policy.clone(),
                Arc::new(ailoop_client.clone().unwrap()),
            );
            self.command_registry.register(ask_command);
        }

        #[cfg(feature = "chat")]
        {
            if self.command_registry.get("chat").is_none() {
                let registry_snapshot = Arc::new(self.command_registry.clone());
                let chat_command = crate::command::create_chat_command(
                    registry_snapshot,
                    self.risk_policy.clone(),
                    ailoop_client.clone().map(Arc::new),
                    self.app_name,
                );
                self.command_registry.register(chat_command);
            } else {
                log::warn!("'chat' command already registered; skipping built-in chat command");
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
                    log::warn!(
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
            log::warn!("'spec' command already registered; skipping built-in spec command");
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

                let serve_cmd = crate::mcp::commands::create_mcp_serve_command_with_deps(
                    registry_arc_for_serve,
                    app_name_for_serve,
                    risk_policy_for_serve,
                    export_policy_for_serve,
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

            if self.command_registry.get("mcp").is_none() {
                let app_name_for_install = self.app_name;

                let install_path = CommandPath::new(&["mcp", "install"]).unwrap();
                if self.command_registry.resolve(&install_path).is_none() {
                    let install_cmd =
                        crate::mcp::commands::create_mcp_install_command(app_name_for_install);
                    let mut register_cmd = install_cmd.clone();
                    register_cmd.id = "register";
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
        }

        let clap_root = crate::app::clap_adapter::build_clap_root(
            self.meta.as_ref(),
            &self.command_registry,
            self.app_name,
            self.app_version,
        );

        let registry_arc = Arc::new(self.command_registry);

        Ok(App {
            command_registry: registry_arc,
            llm_provider: self.llm_provider,
            ailoop_client,
            plugin_registry_manager,
            ctx,
            meta: self.meta,
            app_name: self.app_name,
            app_version: self.app_version,
            clap_root,
            #[cfg(feature = "testkit")]
            stdout_capture: None,
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
    llm_provider: Option<Arc<dyn LlmProvider>>,
    ailoop_client: Option<AiloopClient>,
    plugin_registry_manager: Option<PluginRegistryManager>,
    ctx: C,
    meta: Option<AppMeta>,
    app_name: &'static str,
    app_version: &'static str,
    clap_root: clap::Command,
    /// Captures framework-level stdout output (version strings etc.) when testkit is active.
    #[cfg(feature = "testkit")]
    pub stdout_capture: Option<Arc<Mutex<Vec<u8>>>>,
}

struct CliAppContextWrapper<'a, C: AppContext> {
    _inner: &'a mut C,
    ailoop_client: &'a Option<AiloopClient>,
    command_registry: &'a CommandRegistry,
    llm_provider: &'a Option<Arc<dyn LlmProvider>>,
}

impl<'a, C: AppContext> AppContext for CliAppContextWrapper<'a, C> {
    fn opt_registry(&self) -> Option<&crate::command::CommandRegistry> {
        Some(self.command_registry)
    }

    fn as_any(&self) -> Option<&dyn std::any::Any> {
        self._inner.as_any()
    }

    fn as_any_mut(&mut self) -> Option<&mut dyn std::any::Any> {
        self._inner.as_any_mut()
    }
}

impl<'a, C: AppContext> crate::app::context::LlmContext for CliAppContextWrapper<'a, C> {
    fn llm_provider(&self) -> &dyn crate::llm::LlmProvider {
        self.llm_provider
            .as_ref()
            .expect("LLM provider not configured")
            .as_ref()
    }
}

impl<'a, C: AppContext> crate::app::context::CommandRegistryContext
    for CliAppContextWrapper<'a, C>
{
    fn command_registry(&self) -> &crate::command::CommandRegistry {
        self.command_registry
    }

    fn execute_command_sync(
        &self,
        command_id: &str,
        args: crate::command::CommandArgs,
    ) -> anyhow::Result<()> {
        let command = self
            .command_registry()
            .get(command_id)
            .ok_or_else(|| anyhow::anyhow!("Command '{}' not found", command_id))?
            .clone();

        struct NoopContext;
        impl AppContext for NoopContext {}

        tokio::task::block_in_place(|| {
            tokio::runtime::Handle::current().block_on(async {
                let mut ctx = NoopContext;
                (command.execute)(&mut ctx, args).await
            })
        })
    }
}

impl<'a, C: AppContext> crate::ailoop::AiloopContext for CliAppContextWrapper<'a, C> {
    fn ailoop_client(&self) -> &AiloopClient {
        self.ailoop_client
            .as_ref()
            .expect("Ailoop client not configured")
    }
}

impl<C: AppContext> App<C> {
    #[doc(hidden)]
    pub fn should_show_help(args: &[String]) -> bool {
        args.len() < 2 || args.get(1).is_some_and(|s| s == "--help" || s == "-h")
    }

    pub async fn run(&mut self) -> Result<()> {
        let args: Vec<String> = std::env::args().collect();
        self.run_with_args(args).await
    }

    pub fn rebuild_clap_root(&mut self) {
        self.clap_root = crate::app::clap_adapter::build_clap_root(
            self.meta.as_ref(),
            &self.command_registry,
            self.app_name,
            self.app_version,
        );
    }

    pub async fn run_with_args(&mut self, args: Vec<String>) -> Result<()> {
        use crate::app::clap_adapter::parse_with_clap;
        use crate::app::diagnostic_reporter::DiagnosticReporter;
        use crate::parser::diagnostic::{Diagnostic, DiagnosticCategory};
        use crate::parser::error_codes::E_NESTED_COMMAND_NOT_FOUND;
        use crate::parser::outcome::ParseOutcome;

        match parse_with_clap(&self.clap_root, &self.command_registry, args) {
            ParseOutcome::Parsed {
                command_path,
                args: cmd_args,
                typed_args,
            } => {
                let cmd_id = command_path.leaf().unwrap_or("").to_string();
                if cmd_id == "version" && self.command_registry.get("version").is_none() {
                    if self.app_name == "unknown" {
                        log::warn!("version called but with_version() was not configured");
                    }
                    self.framework_println(&format!("{} {}", self.app_name, self.app_version));
                    return Ok(());
                }

                if command_path.0.len() > 1 {
                    // Multi-segment path: use resolve() for dispatch.
                    match self.command_registry.resolve(&command_path) {
                        Some(cmd) => {
                            let cmd_clone = cmd.clone();
                            self.execute_command_direct(cmd_clone, cmd_args, typed_args)
                                .await
                        }
                        None => {
                            DiagnosticReporter::report(&Diagnostic {
                                code: E_NESTED_COMMAND_NOT_FOUND,
                                category: DiagnosticCategory::Parse,
                                message: format!(
                                    "nested command '{}' not found",
                                    command_path.to_path_string()
                                ),
                                suggestion: Some(
                                    "Use --help to see available commands".to_string(),
                                ),
                                span: None,
                            });
                            Ok(())
                        }
                    }
                } else {
                    self.execute_command_with_typed_args(&cmd_id, cmd_args, typed_args)
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
                DiagnosticReporter::report(&d);
                Ok(())
            }
        }
    }

    /// Write a line of framework-level output. Routes through the testkit capture buffer
    /// when active; otherwise writes to real stdout.
    fn framework_println(&self, s: &str) {
        use std::io::Write;

        #[cfg(feature = "testkit")]
        if let Some(ref buf) = self.stdout_capture {
            let mut lock = buf.lock().unwrap();
            lock.extend_from_slice(s.as_bytes());
            lock.push(b'\n');
            return;
        }

        let mut stdout = std::io::stdout();
        let _ = writeln!(stdout, "{}", s);
    }

    pub fn show_help(&self) {
        println!("  version - Print version information");
        HelpRenderer::new(self.meta.as_ref(), self.command_registry.as_ref()).print();
    }

    pub fn render_help(&self) -> String {
        let mut out = String::from("  version - Print version information\n");
        out.push_str(
            &HelpRenderer::new(self.meta.as_ref(), self.command_registry.as_ref()).render(),
        );
        out
    }

    pub fn version_string(&self) -> String {
        format!("{} {}", self.app_name, self.app_version)
    }

    pub async fn execute_command(
        &mut self,
        command_id: &str,
        args: crate::command::CommandArgs,
    ) -> Result<()> {
        self.execute_command_with_typed_args(command_id, args, None)
            .await
    }

    pub async fn execute_command_with_typed_args(
        &mut self,
        command_id: &str,
        args: crate::command::CommandArgs,
        typed_args: Option<HashMap<String, ArgValue>>,
    ) -> Result<()> {
        let command = self
            .command_registry
            .get(command_id)
            .ok_or_else(|| anyhow::anyhow!("Command '{}' not found", command_id))?
            .clone();
        self.execute_command_direct(command, args, typed_args).await
    }

    /// Execute an already-resolved `Command` with optional typed args.
    /// Shared by both single-segment (`execute_command_with_typed_args`) and
    /// multi-segment dispatch paths in `run_with_args`.
    async fn execute_command_direct(
        &mut self,
        command: Command,
        args: crate::command::CommandArgs,
        typed_args: Option<HashMap<String, ArgValue>>,
    ) -> Result<()> {
        use crate::app::diagnostic_reporter::DiagnosticReporter;

        // Stage 6: Validation Pipeline
        if let (Some(ref spec), Some(ref typed_args_map)) = (&command.spec, &typed_args) {
            // Spec-level validation (E003–E006)
            let spec_diagnostics = SpecValidator::validate(spec, typed_args_map);
            if !spec_diagnostics.is_empty() {
                DiagnosticReporter::report_all(&spec_diagnostics);
                return Err(anyhow::anyhow!("validation failed"));
            }

            // Command-level validation hook (custom validation)
            if let Some(ref validator) = command.validator {
                let custom_diagnostics = validator(typed_args_map);
                if !custom_diagnostics.is_empty() {
                    DiagnosticReporter::report_all(&custom_diagnostics);
                    return Err(anyhow::anyhow!("custom validation failed"));
                }
            }
        }

        let mut ctx_wrapper = CliAppContextWrapper {
            _inner: &mut self.ctx,
            ailoop_client: &self.ailoop_client,
            command_registry: self.command_registry.as_ref(),
            llm_provider: &self.llm_provider,
        };

        // For spec-based commands, build CommandArgs from typed_args so execute closures
        // can access parsed flag values via args.named
        let effective_args = if let Some(ref typed_map) = typed_args {
            let mut named = std::collections::HashMap::new();
            for (k, v) in typed_map {
                use crate::spec::value::ArgValue;
                let s = match v {
                    ArgValue::Bool(b) => b.to_string(),
                    ArgValue::Str(s) => s.clone(),
                    ArgValue::Int(i) => i.to_string(),
                    ArgValue::Float(f) => f.to_string(),
                    ArgValue::Enum(e) => e.clone(),
                    ArgValue::Count(c) => c.to_string(),
                    ArgValue::List(items) => items
                        .iter()
                        .map(|i| match i {
                            ArgValue::Str(s) => s.clone(),
                            ArgValue::Int(i) => i.to_string(),
                            ArgValue::Float(f) => f.to_string(),
                            ArgValue::Enum(e) => e.clone(),
                            _ => String::new(),
                        })
                        .collect::<Vec<_>>()
                        .join(","),
                };
                named.insert(k.clone(), s);
            }
            crate::command::CommandArgs {
                positional: Vec::new(),
                named,
            }
        } else {
            args
        };

        (command.execute)(&mut ctx_wrapper, effective_args).await?;
        Ok(())
    }

    /// Return a reference to the command registry.
    pub fn command_registry(&self) -> &CommandRegistry {
        self.command_registry.as_ref()
    }

    pub fn get_command_metadata(&self) -> Vec<crate::llm::CommandMetadata> {
        self.command_registry.collect_metadata()
    }

    pub fn llm_provider(&self) -> Option<&dyn LlmProvider> {
        self.llm_provider.as_ref().map(|p| p.as_ref())
    }

    pub fn ailoop_client(&self) -> Option<&AiloopClient> {
        self.ailoop_client.as_ref()
    }

    pub fn has_plugins(&self) -> bool {
        self.plugin_registry_manager.is_some()
    }
}
