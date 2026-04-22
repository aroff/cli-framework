//! AppBuilder implementation
//!
//! Provides a builder pattern for constructing CLI applications.

use crate::ailoop::{AiloopClient, AiloopConfig};
use crate::app::context::AppContext;
use crate::app::module::Module;
use crate::app::AppMeta;
use crate::cli_output::HelpRenderer;
use crate::command::{Command, CommandRegistry};
use crate::llm::LlmProvider;
use crate::plugin::PluginRegistryManager;
use anyhow::Result;
use std::path::PathBuf;
use std::sync::Arc;

/// Builder for constructing CLI applications
pub struct AppBuilder {
    command_registry: CommandRegistry,
    plugin_registry_manager: Option<PluginRegistryManager>,
    llm_provider: Option<Arc<dyn LlmProvider>>,
    ailoop_config: Option<AiloopConfig>,
    plugin_registry_path: Option<PathBuf>,
    meta: Option<AppMeta>,
    app_name: &'static str,
    app_version: &'static str,
}

impl AppBuilder {
    /// Create a new AppBuilder with default configuration
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
        }
    }

    /// Register a command
    pub fn register_command(mut self, command: Command) -> Self {
        self.command_registry.register(command);
        self
    }

    /// Register a module
    ///
    /// Modules allow grouping related commands and data sources together.
    /// This method calls the module's `register` method to add its components.
    pub fn register_module<M: Module>(mut self, module: M) -> Result<Self> {
        module.register(&mut self)?;
        Ok(self)
    }

    /// Configure LLM provider for ask command
    pub fn with_llm_provider(mut self, provider: Arc<dyn LlmProvider>) -> Self {
        self.llm_provider = Some(provider);
        self
    }

    /// Configure LLM provider from environment variables
    pub fn with_llm_from_env(mut self) -> Result<Self> {
        let provider = crate::llm::LlmProviderFactory::from_env()?;
        self.llm_provider = Some(provider);
        Ok(self)
    }

    /// Configure ailoop integration for human-in-the-loop interactions
    pub fn with_ailoop_config(mut self, config: AiloopConfig) -> Self {
        self.ailoop_config = Some(config);
        self
    }

    /// Configure ailoop channel (convenience method)
    pub fn with_ailoop_channel(self, channel: &str) -> Self {
        let config = AiloopConfig {
            channel: channel.to_string(),
            server_url: None,
            default_timeout_seconds: 300,
        };
        self.with_ailoop_config(config)
    }

    /// Configure plugin registry path
    pub fn with_plugin_registry_path(mut self, path: PathBuf) -> Self {
        self.plugin_registry_path = Some(path.clone());
        self.plugin_registry_manager = Some(PluginRegistryManager::new(path));
        self
    }

    /// Set application-level metadata used in the help header.
    pub fn with_meta(mut self, meta: AppMeta) -> Self {
        self.meta = Some(meta);
        self
    }

    /// Set the application name and version reported by the `version` command.
    ///
    /// Recommended: pass `env!("CARGO_PKG_NAME")` and `env!("CARGO_PKG_VERSION")`
    /// so the values are resolved at the *calling crate's* compile time.
    pub fn with_version(mut self, name: &'static str, version: &'static str) -> Self {
        self.app_name = name;
        self.app_version = version;
        self
    }

    /// Build the CLI application
    pub fn build<C: AppContext + 'static>(mut self, ctx: C) -> Result<App<C>> {
        // Initialize ailoop client if configured
        let ailoop_client = if let Some(config) = self.ailoop_config {
            Some(AiloopClient::with_config(config)?)
        } else {
            None
        };

        // Initialize plugin registry manager if configured
        let plugin_registry_manager = self.plugin_registry_manager;

        // Note: Plugin loading is deferred until App::run() is called

        // Register "ask" command if LLM provider is configured
        if let Some(ref provider) = self.llm_provider {
            let ask_command = crate::command::create_ask_command(provider.clone());
            self.command_registry.register(ask_command);
        }

        Ok(App {
            command_registry: self.command_registry,
            llm_provider: self.llm_provider,
            ailoop_client,
            plugin_registry_manager,
            ctx,
            meta: self.meta,
            app_name: self.app_name,
            app_version: self.app_version,
        })
    }
}

impl Default for AppBuilder {
    fn default() -> Self {
        Self::new()
    }
}

/// Built CLI application
pub struct App<C: AppContext> {
    command_registry: CommandRegistry,
    llm_provider: Option<Arc<dyn LlmProvider>>,
    ailoop_client: Option<AiloopClient>,
    plugin_registry_manager: Option<PluginRegistryManager>,
    ctx: C,
    meta: Option<AppMeta>,
    app_name: &'static str,
    app_version: &'static str,
}

/// Context wrapper that provides access to CLI framework services
struct CliAppContextWrapper<'a, C: AppContext> {
    _inner: &'a mut C,
    ailoop_client: &'a Option<AiloopClient>,
    command_registry: &'a CommandRegistry,
    llm_provider: &'a Option<Arc<dyn LlmProvider>>,
}

impl<'a, C: AppContext> AppContext for CliAppContextWrapper<'a, C> {
    // Delegate to inner context for any custom methods
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
        _command_id: &str,
        _args: crate::command::CommandArgs,
    ) -> anyhow::Result<()> {
        // In a real implementation, this would execute the command synchronously or spawn it
        // For now, this is a placeholder to satisfy the trait
        Err(anyhow::anyhow!(
            "Synchronous command execution not yet implemented"
        ))
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

    /// Run the CLI application
    ///
    /// This parses command-line arguments and executes the corresponding command.
    pub async fn run(&mut self) -> Result<()> {
        let args: Vec<String> = std::env::args().collect();
        self.run_with_args(args).await
    }

    /// Run the CLI application with the given arguments.
    ///
    /// This is the testable equivalent of [`run`](Self::run) that accepts
    /// an explicit argument list instead of reading `std::env::args()`.
    pub async fn run_with_args(&mut self, args: Vec<String>) -> Result<()> {
        if Self::should_show_help(&args) {
            self.show_help();
            return Ok(());
        }

        match args[1].as_str() {
            "version" | "--version" => {
                if self.app_name == "unknown" {
                    log::warn!("version called but with_version() was not configured");
                }
                println!("{} {}", self.app_name, self.app_version);
                return Ok(());
            }
            _ => {}
        }

        let command_id = &args[1];
        let remaining_args = &args[2..];

        let mut positional = Vec::new();
        let mut named = std::collections::HashMap::new();

        let mut i = 0;
        while i < remaining_args.len() {
            let arg = &remaining_args[i];
            if arg.starts_with("--") {
                let key = arg.trim_start_matches("--").to_string();
                if i + 1 < remaining_args.len() && !remaining_args[i + 1].starts_with("--") {
                    named.insert(key, remaining_args[i + 1].clone());
                    i += 2;
                } else {
                    named.insert(key, "true".to_string());
                    i += 1;
                }
            } else {
                positional.push(arg.clone());
                i += 1;
            }
        }

        let cmd_args = crate::command::CommandArgs { positional, named };
        self.execute_command(command_id, cmd_args).await
    }

    /// Show help information
    pub fn show_help(&self) {
        println!("  version - Print version information");
        HelpRenderer::new(self.meta.as_ref(), &self.command_registry).print();
    }

    /// Return the full help output as a String (for testing).
    pub fn render_help(&self) -> String {
        let mut out = String::from("  version - Print version information\n");
        out.push_str(&HelpRenderer::new(self.meta.as_ref(), &self.command_registry).render());
        out
    }

    /// Return the version string that would be printed by the version command.
    pub fn version_string(&self) -> String {
        format!("{} {}", self.app_name, self.app_version)
    }

    /// Execute a command by ID
    ///
    /// This method looks up and executes a command with the given arguments.
    pub async fn execute_command(
        &mut self,
        command_id: &str,
        args: crate::command::CommandArgs,
    ) -> Result<()> {
        let command = self
            .command_registry
            .get(command_id)
            .ok_or_else(|| anyhow::anyhow!("Command '{}' not found", command_id))?;

        // Create a context wrapper that includes CLI app services
        let mut ctx_wrapper = CliAppContextWrapper {
            _inner: &mut self.ctx,
            ailoop_client: &self.ailoop_client,
            command_registry: &self.command_registry,
            llm_provider: &self.llm_provider,
        };

        // Execute the command
        (command.execute)(&mut ctx_wrapper, args).await?;
        Ok(())
    }

    /// Get available commands metadata for LLM context
    pub fn get_command_metadata(&self) -> Vec<crate::llm::CommandMetadata> {
        self.command_registry.collect_metadata()
    }

    /// Get LLM provider if configured
    pub fn llm_provider(&self) -> Option<&dyn LlmProvider> {
        self.llm_provider.as_ref().map(|p| p.as_ref())
    }

    /// Get ailoop client if configured
    pub fn ailoop_client(&self) -> Option<&AiloopClient> {
        self.ailoop_client.as_ref()
    }

    /// Check if plugins are configured
    pub fn has_plugins(&self) -> bool {
        self.plugin_registry_manager.is_some()
    }
}
