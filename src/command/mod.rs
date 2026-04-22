pub mod ask;
pub mod parser;
pub mod registry;

pub use ask::create_ask_command;
pub use registry::CommandRegistry;

use crate::app::context::AppContext;
use anyhow::Result;
use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

/// Command identifier
pub type CommandId = &'static str;

/// Command arguments (positional and named)
#[derive(Debug, Clone, Default)]
pub struct CommandArgs {
    /// Positional arguments
    pub positional: Vec<String>,
    /// Named arguments (key-value pairs)
    pub named: HashMap<String, String>,
}

/// Command result type
pub type CommandResult = Result<()>;

/// Command struct representing an executable operation
///
/// # Async Execution
///
/// The `execute` function is async, allowing commands to perform async operations
/// (network requests, database queries, etc.) using `.await` without blocking the UI.
#[derive(Clone)]
pub struct Command {
    /// Unique command identifier
    pub id: CommandId,
    /// Short description (shown in command palette)
    pub summary: &'static str,
    /// Optional syntax hint (e.g., ":restart service=<name> env=<env>")
    pub syntax: Option<&'static str>,
    /// Optional category for grouping in palette
    pub category: Option<&'static str>,
    /// Execution function (async)
    ///
    /// Returns a boxed future that will be awaited by the framework.
    /// Uses `Arc<dyn Fn>` to allow closures that capture state (e.g. the ask command).
    #[allow(clippy::type_complexity)]
    pub execute: Arc<
        dyn Fn(
                &mut dyn AppContext,
                CommandArgs,
            ) -> Pin<Box<dyn Future<Output = CommandResult> + Send>>
            + Send
            + Sync,
    >,
}
