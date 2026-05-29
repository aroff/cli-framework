#[cfg(feature = "chat")]
pub mod chat;
pub mod parser;
pub mod registry;

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

/// Command identifier
pub type CommandId = &'static str;

/// Command arguments (positional and named)
#[derive(Debug, Clone, Default)]
pub struct CommandArgs {
    /// Positional arguments
    pub positional: Vec<String>,
    /// Named arguments (key-value pairs, legacy string representation)
    pub named: HashMap<String, String>,
    /// Typed named arguments populated when a CommandSpec is present
    pub named_typed: HashMap<String, ArgValue>,
}

impl CommandArgs {
    pub fn get_str(&self, key: &str) -> Option<&str> {
        self.named_typed
            .get(key)
            .and_then(|v| {
                if let ArgValue::Str(s) = v {
                    Some(s.as_str())
                } else {
                    None
                }
            })
            .or_else(|| self.named.get(key).map(String::as_str))
    }

    pub fn get_bool(&self, key: &str) -> bool {
        match self.named_typed.get(key) {
            Some(ArgValue::Bool(b)) => *b,
            _ => self.named.get(key).map(|s| s == "true").unwrap_or(false),
        }
    }

    pub fn get_int(&self, key: &str) -> Option<i64> {
        match self.named_typed.get(key) {
            Some(ArgValue::Int(i)) => Some(*i),
            _ => self.named.get(key).and_then(|s| s.parse().ok()),
        }
    }

    pub fn get_float(&self, key: &str) -> Option<f64> {
        match self.named_typed.get(key) {
            Some(ArgValue::Float(fl)) => Some(*fl),
            _ => self.named.get(key).and_then(|s| s.parse().ok()),
        }
    }

    pub fn get_list(&self, key: &str) -> Vec<ArgValue> {
        match self.named_typed.get(key) {
            Some(ArgValue::List(items)) => items.clone(),
            _ => vec![],
        }
    }
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
