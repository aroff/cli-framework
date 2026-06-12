#[cfg(feature = "chat")]
pub mod chat;
pub mod parser;
pub mod registry;
pub mod typed;

#[cfg(feature = "chat")]
pub use chat::create_chat_command;
pub use registry::CommandRegistry;
pub use typed::{FromArgValueMap, IntoCommandSpec, TypedArgs};

use crate::app::context::AppContext;
use crate::parser::diagnostic::Diagnostic;
use crate::spec::command_tree::CommandSpec;
use crate::spec::value::ArgValue;
use anyhow::Result;
use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

/// Command identifier — the leaf name used as the registry key.
pub type CommandId = Arc<str>;

/// Command result type
pub type CommandResult = Result<()>;

/// A registered executable command.
///
/// All metadata (summary, category, syntax, args) lives in `spec`.
/// The execute closure receives a fully-validated typed ArgValue map.
#[derive(Clone)]
pub struct Command {
    /// Leaf name / registry key (e.g. "run", "serve").
    pub id: CommandId,
    /// Mandatory typed spec. Carries all command metadata.
    pub spec: Arc<CommandSpec>,
    /// Optional command-level cross-field validation hook.
    #[allow(clippy::type_complexity)]
    pub validator: Option<Arc<dyn Fn(&HashMap<String, ArgValue>) -> Vec<Diagnostic> + Send + Sync>>,
    /// Async execute function.
    /// Receives a validated ArgValue map produced by map_matches_to_typed_args.
    #[allow(clippy::type_complexity)]
    pub execute: Arc<
        dyn for<'a> Fn(
                &'a mut dyn AppContext,
                HashMap<String, ArgValue>,
            ) -> Pin<Box<dyn Future<Output = CommandResult> + Send + 'a>>
            + Send
            + Sync,
    >,
    /// Whether this command is eligible for MCP tool export.
    pub expose_mcp: bool,
    /// Whether this command appears in the chat agent's tool list.
    ///
    /// Default: `true` (opt-out model — commands are visible unless explicitly excluded).
    /// Set to `false` on framework-internal commands that should never be called by an LLM.
    pub expose_chat: bool,
    /// Optional opaque per-tool `_meta` passthrough. When set, the emitted MCP
    /// tool carries this value verbatim as its top-level `_meta` on the
    /// `tools/list` entry. cli-framework never inspects its contents — the
    /// consumer owns the entire shape.
    pub meta: Option<serde_json::Value>,
    /// Optional MCP visibility tags (e.g. `["app"]`) emitted on the tool.
    ///
    /// cli-framework *acts on* this: `Some(vec!["app"])` marks an *app-only*
    /// tool that remains dispatchable via `tools/call` but is flagged (in
    /// `_meta.visibility`) so hosts can hide it from the model.
    pub visibility: Option<Vec<String>>,
}

impl Command {
    pub fn summary(&self) -> &'static str {
        self.spec.summary
    }

    pub fn syntax(&self) -> Option<&'static str> {
        self.spec.syntax
    }

    pub fn category(&self) -> Option<&'static str> {
        self.spec.category
    }

    /// Attach a command-level validation hook.
    pub fn with_validator<F>(mut self, f: F) -> Self
    where
        F: Fn(&HashMap<String, ArgValue>) -> Vec<Diagnostic> + Send + Sync + 'static,
    {
        self.validator = Some(Arc::new(f));
        self
    }

    /// Set whether this command appears in MCP tool listings.
    pub fn with_expose_mcp(mut self, enabled: bool) -> Self {
        self.expose_mcp = enabled;
        self
    }

    /// Set whether this command appears in the chat agent's tool list.
    /// Default: `true` (opt-out model; commands are visible unless explicitly excluded).
    pub fn with_expose_chat(mut self, enabled: bool) -> Self {
        self.expose_chat = enabled;
        self
    }

    /// Attach an opaque per-tool `_meta` value. The emitted MCP tool carries
    /// this value verbatim as its top-level `_meta` in `tools/list`.
    ///
    /// cli-framework treats the value as opaque passthrough and never inspects
    /// it; the consumer owns its shape.
    pub fn with_meta(mut self, meta: serde_json::Value) -> Self {
        self.meta = Some(meta);
        self
    }

    /// Set MCP visibility tags (e.g. `["app"]`) for this command.
    ///
    /// `["app"]` marks an app-only tool: still dispatchable via `tools/call`,
    /// but flagged (in `_meta.visibility`) so hosts can hide it from the model.
    pub fn with_visibility(mut self, visibility: Vec<String>) -> Self {
        self.visibility = Some(visibility);
        self
    }
}
