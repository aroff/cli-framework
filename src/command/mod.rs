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

/// Content-Security-Policy directives applied to a UI resource when it is
/// rendered inside an MCP-Apps host iframe.
///
/// Each field maps to a CSP directive; `None` omits the directive. Serialized
/// in `kebab-case` so the wire form matches the directive names exactly
/// (`default-src`, `style-src`, `img-src`, …) as required by the MCP-Apps
/// `_meta.ui.csp` contract.
#[derive(Debug, Clone, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct UiCsp {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default_src: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub style_src: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub script_src: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub img_src: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub connect_src: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub font_src: Option<String>,
}

/// UI metadata attached to a command so its emitted MCP tool advertises an
/// associated MCP-Apps view resource.
///
/// Surfaces on the tool's `tools/list` entry as `_meta.ui` (see
/// `command_to_tool_descriptor`). The `resource_uri` points at a `ui://…`
/// resource served by the framework's [`crate::mcp`] resource registry (CF-2).
#[derive(Debug, Clone, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UiToolMeta {
    /// The `ui://…` resource URI this command's view is served from.
    pub resource_uri: String,
    /// Optional per-view Content-Security-Policy advertised to the host.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub csp: Option<UiCsp>,
    /// Hint to the host that it should prefer opening the app view over the
    /// text fallback. Always serialized (it is a meaningful `false`).
    #[serde(default)]
    pub prefer_app: bool,
}

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
    /// Optional MCP-Apps UI metadata. When set, the emitted tool advertises an
    /// associated `ui://…` view resource via `_meta.ui` in `tools/list` (CF-1).
    pub ui: Option<UiToolMeta>,
    /// Optional MCP-Apps visibility tags (e.g. `["app"]`) emitted on the tool.
    ///
    /// `Some(vec!["app"])` marks an *app-only* tool: it remains dispatchable via
    /// `tools/call` but is flagged so hosts can hide it from the model (CF-3).
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

    /// Attach MCP-Apps UI metadata. The emitted tool advertises the associated
    /// `ui://…` view resource via `_meta.ui` in `tools/list` (CF-1).
    pub fn with_ui(mut self, meta: UiToolMeta) -> Self {
        self.ui = Some(meta);
        self
    }

    /// Set MCP-Apps visibility tags (e.g. `["app"]`) for this command.
    ///
    /// `["app"]` marks an app-only tool: still dispatchable via `tools/call`,
    /// but flagged so hosts can hide it from the model (CF-3).
    pub fn with_visibility(mut self, visibility: Vec<String>) -> Self {
        self.visibility = Some(visibility);
        self
    }
}
