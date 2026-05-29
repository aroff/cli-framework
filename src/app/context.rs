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

    /// Optional downcasting support for commands that need access to concrete app context types.
    ///
    /// Framework-internal wrapper contexts are not `'static` and therefore cannot support
    /// `Any` downcasting; they return `None` by default.
    fn as_any(&self) -> Option<&dyn std::any::Any> {
        None
    }

    /// Optional mutable downcasting support for commands that need to mutate concrete context state.
    fn as_any_mut(&mut self) -> Option<&mut dyn std::any::Any> {
        None
    }

    /// Write a line of user-visible output.
    ///
    /// Commands should prefer this over `println!` so framework consumers can
    /// capture output deterministically in tests.
    fn framework_println(&self, s: &str) {
        use std::io::Write;
        let mut stdout = std::io::stdout();
        let _ = writeln!(stdout, "{}", s);
    }
}

/// Extension trait for AppContext to provide command registry access
pub trait CommandRegistryContext {
    /// Get the command registry for command lookup and metadata
    fn command_registry(&self) -> &crate::command::CommandRegistry;

    /// Execute another command by ID
    fn execute_command_sync(
        &self,
        command_id: &str,
        args: crate::command::CommandArgs,
    ) -> anyhow::Result<()>;
}
