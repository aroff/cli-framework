pub mod ask;
#[cfg(feature = "chat")]
pub mod chat;
pub mod parser;
pub mod registry;

pub use ask::create_ask_command;
#[cfg(feature = "chat")]
pub use chat::create_chat_command;
pub use registry::CommandRegistry;

use crate::app::context::AppContext;
use crate::parser::diagnostic::Diagnostic;
use crate::spec::command_tree::CommandSpec;
use crate::spec::value::ArgValue;
use anyhow::Result;
use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

/// Shared risk-gate preflight used by `ask` and `chat`.
///
/// This function is retained as a stable entrypoint for callers/tests, but the
/// canonical implementation lives in `crate::security::RiskEnforcer`.
pub fn enforce_risk_gate(
    policy: &crate::security::command_risk::CommandRiskPolicy,
    resolution: &crate::llm::CommandResolution,
    command_category: Option<&str>,
    assume_yes: bool,
    ailoop_available: bool,
) -> anyhow::Result<()> {
    crate::security::RiskEnforcer::new(policy.clone()).enforce_preflight(
        &resolution.command_id,
        command_category,
        assume_yes,
        ailoop_available,
    )
}

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
    /// Typed argument spec; `None` preserves legacy behavior.
    pub spec: Option<Arc<CommandSpec>>,
    /// Optional command-level validation hook (Stage 6).
    #[allow(clippy::type_complexity)]
    pub validator: Option<Arc<dyn Fn(&HashMap<String, ArgValue>) -> Vec<Diagnostic> + Send + Sync>>,
    /// Execution function (async)
    ///
    /// Returns a boxed future that will be awaited by the framework.
    /// Uses `Arc<dyn Fn>` to allow closures that capture state (e.g. the ask command).
    #[allow(clippy::type_complexity)]
    pub execute: Arc<
        dyn for<'a> Fn(
                &'a mut dyn AppContext,
                CommandArgs,
            ) -> Pin<Box<dyn Future<Output = CommandResult> + Send + 'a>>
            + Send
            + Sync,
    >,
    /// Whether this command is eligible for MCP tool export.
    /// Ignored under `McpToolExportPolicy::AllCommands` (the default).
    /// Under `McpToolExportPolicy::ExposeMcpOnly`, only commands with
    /// `expose_mcp == true` are registered as MCP tools.
    pub expose_mcp: bool,
}

impl Command {
    /// Attach a typed `CommandSpec` to this command.
    pub fn with_spec(mut self, spec: CommandSpec) -> Self {
        self.spec = Some(Arc::new(spec));
        self
    }

    /// Set whether this command appears in MCP tool listings.
    pub fn with_expose_mcp(mut self, enabled: bool) -> Self {
        self.expose_mcp = enabled;
        self
    }

    /// Attach a command-level validation hook.
    pub fn with_validator<F>(mut self, f: F) -> Self
    where
        F: Fn(&HashMap<String, ArgValue>) -> Vec<Diagnostic> + Send + Sync + 'static,
    {
        self.validator = Some(Arc::new(f));
        self
    }
}
