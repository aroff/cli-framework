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

    /// Write a line of user-visible output.
    ///
    /// Commands should prefer this over `println!` so framework consumers can
    /// capture output deterministically in tests.
    fn framework_println(&self, s: &str) {
        use std::io::Write;
        let mut stdout = std::io::stdout();
        let _ = writeln!(stdout, "{}", s);
    }

    /// Drain and return any output captured since the last call.
    ///
    /// Contexts that capture `framework_println` output override this to return
    /// and clear the internal buffer. The default returns an empty string.
    fn drain_output(&self) -> String {
        String::new()
    }

    /// Return the global args parsed for the current invocation, if available.
    ///
    /// Returns `None` for contexts that do not carry global args (e.g., user-defined
    /// contexts outside the dispatch path). The dispatch wrapper always provides `Some`.
    fn opt_global_args(
        &self,
    ) -> Option<&std::collections::HashMap<String, crate::spec::value::ArgValue>> {
        None
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
        args: std::collections::HashMap<String, crate::spec::value::ArgValue>,
    ) -> anyhow::Result<()>;
}
