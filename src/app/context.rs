//! AppContext trait and helpers
//!
//! AppContext represents application-owned state and service clients.

/// Application context trait
///
/// Applications implement this trait to provide their own state and service clients.
/// The framework uses this to pass context to views, datasources, and commands.
///
/// # Thread Safety
///
/// AppContext implementations must be `Send + Sync` to ensure thread safety in the async runtime.
/// This allows the framework to safely share context across async tasks and threads.
pub trait AppContext: Send + Sync {
    // Applications define their own structure
    // Framework only requires the trait to exist

    /// Provides access to the frozen command registry, populated by `AppBuilder::build`.
    /// Returns `None` for contexts that do not expose the registry (e.g., user-defined contexts).
    fn opt_registry(&self) -> Option<&crate::command::CommandRegistry> {
        None
    }
}

/// Extension trait for AppContext to provide LLM provider access
pub trait LlmContext {
    /// Get the LLM provider for command resolution
    fn llm_provider(&self) -> &dyn crate::llm::LlmProvider;
}

/// Extension trait for AppContext to provide command registry access
pub trait CommandRegistryContext {
    /// Get the command registry for command lookup and metadata
    fn command_registry(&self) -> &crate::command::CommandRegistry;

    /// Execute another command by ID
    ///
    /// This allows commands (like "ask") to trigger other commands.
    fn execute_command_sync(
        &self,
        command_id: &str,
        args: crate::command::CommandArgs,
    ) -> anyhow::Result<()>;
}
