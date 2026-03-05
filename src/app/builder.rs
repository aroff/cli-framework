//! AppBuilder implementation
//!
//! Provides a builder pattern for constructing CLI applications.

use crate::ailoop::{AiloopClient, AiloopConfig, AiloopContext};
use crate::app::context::AppContext;
use crate::app::module::Module;
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
    llm_provider: Option<Box<dyn LlmProvider>>,
    ailoop_config: Option<AiloopConfig>,
    plugin_registry_path: Option<PathBuf>,
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
    pub fn with_llm_provider(mut self, provider: Box<dyn LlmProvider>) -> Self {
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

    /// Build the CLI application
    pub fn build<C: AppContext + 'static>(mut self, ctx: C) -> Result<App<C>> {
        // Initialize ailoop client if configured
        let ailoop_client = if let Some(config) = self.ailoop_config {
            Some(AiloopClient::with_config(config)?)
        } else {
            None
        };

        // Initialize plugin registry manager if configured
        let mut plugin_registry_manager = self.plugin_registry_manager;

        // Note: Plugin loading is deferred until App::run() is called

        // Note: Ask command registration is deferred until proper LLM provider handling is implemented

        Ok(App {
            command_registry: self.command_registry,
            llm_provider: self.llm_provider,
            ailoop_client,
            plugin_registry_manager,
            ctx,
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
    llm_provider: Option<Box<dyn LlmProvider>>,
    ailoop_client: Option<AiloopClient>,
    plugin_registry_manager: Option<PluginRegistryManager>,
    ctx: C,
}

/// Context wrapper that provides access to CLI framework services
struct CliAppContextWrapper<'a, C: AppContext> {
    inner: &'a mut C,
    ailoop_client: &'a Option<AiloopClient>,
}

impl<'a, C: AppContext> AppContext for CliAppContextWrapper<'a, C> {
    // Delegate to inner context for any custom methods
}

impl<C: AppContext> App<C> {
    /// Execute a command by ID
    ///
    /// This method looks up and executes a command with the given arguments.
    pub async fn execute_command(&mut self, command_id: &str, args: crate::command::CommandArgs) -> Result<()> {
        let command = self.command_registry.get(command_id)
            .ok_or_else(|| anyhow::anyhow!("Command '{}' not found", command_id))?;

        // Create a context wrapper that includes CLI app services
        let mut ctx_wrapper = CliAppContextWrapper {
            inner: &mut self.ctx,
            ailoop_client: &self.ailoop_client,
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

