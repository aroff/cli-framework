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

    pub fn register_command(mut self, command: Command) -> Self {
        self.command_registry.register(command);
        self
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

    pub fn build<C: AppContext + 'static>(mut self, ctx: C) -> Result<App<C>> {
        let ailoop_client = if let Some(config) = self.ailoop_config {
            Some(AiloopClient::with_config(config)?)
        } else {
            None
        };

        let plugin_registry_manager = self.plugin_registry_manager;

        if let Some(ref provider) = self.llm_provider {
            let registry_snapshot = Arc::new(self.command_registry.clone());
            let ask_command =
                crate::command::create_ask_command(provider.clone(), registry_snapshot);
            self.command_registry.register(ask_command);
        }

        #[cfg(feature = "clap-dispatch")]
        let clap_root = crate::app::clap_adapter::build_clap_root(
            self.meta.as_ref(),
            &self.command_registry,
            self.app_name,
            self.app_version,
        );

        Ok(App {
            command_registry: Arc::new(self.command_registry),
            llm_provider: self.llm_provider,
            ailoop_client,
            plugin_registry_manager,
            ctx,
            meta: self.meta,
            app_name: self.app_name,
            app_version: self.app_version,
            #[cfg(feature = "clap-dispatch")]
            clap_root,
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
    #[cfg(feature = "clap-dispatch")]
    clap_root: clap::Command,
}

struct CliAppContextWrapper<'a, C: AppContext> {
    _inner: &'a mut C,
    ailoop_client: &'a Option<AiloopClient>,
    command_registry: &'a CommandRegistry,
    llm_provider: &'a Option<Arc<dyn LlmProvider>>,
}

impl<'a, C: AppContext> AppContext for CliAppContextWrapper<'a, C> {}

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

    #[cfg(feature = "clap-dispatch")]
    pub fn rebuild_clap_root(&mut self) {
        self.clap_root = crate::app::clap_adapter::build_clap_root(
            self.meta.as_ref(),
            &self.command_registry,
            self.app_name,
            self.app_version,
        );
    }

    #[cfg(feature = "clap-dispatch")]
    pub async fn run_with_args(&mut self, args: Vec<String>) -> Result<()> {
        if let Some(parsed) = crate::app::clap_adapter::parse_with_clap(&self.clap_root, args)? {
            if parsed.command_id == "version" && self.command_registry.get("version").is_none() {
                if self.app_name == "unknown" {
                    log::warn!("version called but with_version() was not configured");
                }
                println!("{} {}", self.app_name, self.app_version);
                return Ok(());
            }
            self.execute_command(&parsed.command_id, parsed.args).await
        } else {
            Ok(())
        }
    }

    #[cfg(not(feature = "clap-dispatch"))]
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
        let command = self
            .command_registry
            .get(command_id)
            .ok_or_else(|| anyhow::anyhow!("Command '{}' not found", command_id))?
            .clone();

        let mut ctx_wrapper = CliAppContextWrapper {
            _inner: &mut self.ctx,
            ailoop_client: &self.ailoop_client,
            command_registry: self.command_registry.as_ref(),
            llm_provider: &self.llm_provider,
        };

        (command.execute)(&mut ctx_wrapper, args).await?;
        Ok(())
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
