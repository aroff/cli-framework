use crate::ailoop::AiloopClient;
use crate::command::{Command, CommandRegistry, CommandResult};
use crate::security::command_risk::CommandRiskPolicy;
use crate::spec::arg_spec::{ArgKind, ArgSpec, ArgValueType, Cardinality};
use crate::spec::command_tree::{CommandSpec, EnvVarEntry, ExitCodeEntry};
use crate::spec::value::ArgValue;
use std::collections::HashMap;
use std::sync::Arc;

/// Controls which commands the in-process chat agent may call as tools.
///
/// Set via [`crate::app::AppBuilder::with_chat_tool_policy`]. Default: [`ChatToolPolicy::All`].
#[allow(clippy::type_complexity)]
#[derive(Clone, Default)]
pub enum ChatToolPolicy {
    /// Expose every command regardless of `expose_chat` (backward-compatible default).
    /// Produces a tool list identical to today's hardcoded `AllCommands` behavior.
    #[default]
    All,
    /// Expose only commands where `Command::expose_chat == true`.
    UseCommandFlag,
    /// Caller-supplied predicate for full control at build time.
    /// Arguments: (`path_str`: slash-joined registry key, `command`: resolved `Command`).
    /// Return `true` to include the command in the tool list.
    Custom(Arc<dyn Fn(&str, &Command) -> bool + Send + Sync>),
}

impl std::fmt::Debug for ChatToolPolicy {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::All => write!(f, "All"),
            Self::UseCommandFlag => write!(f, "UseCommandFlag"),
            Self::Custom(_) => write!(f, "Custom(<fn>)"),
        }
    }
}

pub mod host_tool_adapter;
mod runtime;

pub const CHAT_FEATURE_DISABLED: &str = "CHAT_FEATURE_DISABLED";
pub const CHAT_AGENT_START_FAILED: &str = "CHAT_AGENT_START_FAILED";
pub const CHAT_TOOL_NOT_FOUND: &str = "CHAT_TOOL_NOT_FOUND";
pub const CHAT_ARG_VALIDATION_FAILED: &str = "CHAT_ARG_VALIDATION_FAILED";
pub const CHAT_COMMAND_EXECUTION_FAILED: &str = "CHAT_COMMAND_EXECUTION_FAILED";
pub const CHAT_RISK_REQUIRES_CONFIRMATION: &str = "CHAT_RISK_REQUIRES_CONFIRMATION";
pub const CHAT_DESTRUCTIVE_BLOCKED: &str = "CHAT_DESTRUCTIVE_BLOCKED";
pub const CHAT_TOOL_REGISTRY_COLLISION: &str = "CHAT_TOOL_REGISTRY_COLLISION";

#[derive(Debug, Clone)]
pub struct ChatToolCallOptions {
    pub yolo: bool,
    pub interactive: bool,
    pub ailoop_client: Option<Arc<AiloopClient>>,
}

pub fn create_chat_command(
    registry: Arc<CommandRegistry>,
    risk_policy: CommandRiskPolicy,
    ailoop_client: Option<Arc<AiloopClient>>,
    app_name: &'static str,
    chat_tool_policy: ChatToolPolicy,
) -> Command {
    Command {
        id: Arc::from("chat"),
        spec: Arc::new(chat_spec()),
        validator: None,
        expose_mcp: false,
        expose_chat: false,
        execute: Arc::new(move |ctx, args| {
            let client = ailoop_client.clone();
            let registry = Arc::clone(&registry);
            let risk_policy = risk_policy.clone();
            let policy = chat_tool_policy.clone();
            Box::pin(async move {
                execute_chat(ctx, registry, risk_policy, client, app_name, policy, args).await
            })
        }),
    }
}

fn chat_spec() -> CommandSpec {
    CommandSpec {
        summary: "In-process chat session (commands-as-tools)",
        syntax: Some(
            "chat [-p <prompt>] [--stream] [--yolo] [--model <model>] [--session-agents <agents>]",
        ),
        category: Some("ai"),
        long_about: Some(
            "Modes:\n\
 - One-shot: provide a prompt with --prompt/-p or pipe stdin.\n\
 - REPL: run with no prompt and stdin is a TTY.\n\
\n\
Exit:\n\
 - REPL exits on EOF (Ctrl+D) or Ctrl+C (SIGINT). Ctrl+C cancels any in-flight turn.\n\
\n\
Notes:\n\
 - Tool calls are serialized (one command executes at a time).\n\
 - Tools are limited to this process's registered CLI commands.\n\
 - Ctrl+C cancellation is best-effort; in-flight HTTP requests are cancelled via dropping the request future.\n\
 - LLM HTTP timeouts and base URL come from AIKIT_* env configuration.\n\
 - --stream enables server-side streaming, but output is still printed once per turn (no structured event stream).\n\
 - Structured output modes (events/JSON) are not enabled in this rollout phase.\n\
 - LLM configuration is resolved from environment variables (OPENAI_API_KEY / AIKIT_*).\n\
",
        ),
        env_vars: vec![
            EnvVarEntry {
                name: "ALLOW_DESTRUCTIVE_COMMANDS",
                description:
                    "Set to 1/true to allow destructive commands (still requires confirmation)",
            },
            EnvVarEntry {
                name: "OPENAI_API_KEY",
                description: "API key used by the embedded chat agent (OpenAI-compatible)",
            },
            EnvVarEntry {
                name: "AIKIT_LLM_URL",
                description: "OpenAI-compatible base URL (default: https://api.openai.com/v1)",
            },
            EnvVarEntry {
                name: "AIKIT_MODEL",
                description: "Default model name (overridden by --model)",
            },
        ],
        exit_codes: vec![
            ExitCodeEntry {
                code: 1,
                description: "CHAT_AGENT_START_FAILED: embedded agent initialization/config error",
            },
            ExitCodeEntry {
                code: 2,
                description: "CHAT_ARG_VALIDATION_FAILED: tool call arguments invalid",
            },
        ],
        args: vec![
            ArgSpec {
                name: "prompt",
                kind: ArgKind::Option,
                short: Some('p'),
                long: Some("prompt"),
                value_type: ArgValueType::String,
                cardinality: Cardinality::Optional,
                default: None,
                conflicts_with: vec![],
                requires: vec![],
                help: "Prompt text (if omitted, reads stdin or starts a REPL)",
                ..Default::default()
            },
            ArgSpec {
                name: "model",
                kind: ArgKind::Option,
                short: Some('m'),
                long: Some("model"),
                value_type: ArgValueType::String,
                cardinality: Cardinality::Optional,
                default: None,
                conflicts_with: vec![],
                requires: vec![],
                help: "Model override (OpenAI-compatible; best-effort)",
                ..Default::default()
            },
            ArgSpec {
                name: "yolo",
                kind: ArgKind::Flag,
                short: None,
                long: Some("yolo"),
                value_type: ArgValueType::Bool,
                cardinality: Cardinality::Optional,
                default: None,
                conflicts_with: vec![],
                requires: vec![],
                help:
                    "Skip confirmations for Sensitive commands (does not bypass Destructive gating)",
                ..Default::default()
            },
            ArgSpec {
                name: "stream",
                kind: ArgKind::Flag,
                short: None,
                long: Some("stream"),
                value_type: ArgValueType::Bool,
                cardinality: Cardinality::Optional,
                default: None,
                conflicts_with: vec![],
                requires: vec![],
                help: "Stream agent output (best-effort; line-oriented)",
                ..Default::default()
            },
            ArgSpec {
                name: "session_agents",
                kind: ArgKind::Option,
                short: None,
                long: Some("session-agents"),
                value_type: ArgValueType::String,
                cardinality: Cardinality::Optional,
                default: None,
                conflicts_with: vec![],
                requires: vec![],
                help: "Comma-separated agent persona IDs (reserved; best-effort)",
                ..Default::default()
            },
        ],
        ..Default::default()
    }
}

async fn execute_chat(
    ctx: &mut dyn crate::app::context::AppContext,
    registry_fallback: Arc<CommandRegistry>,
    risk_policy: CommandRiskPolicy,
    ailoop_client: Option<Arc<AiloopClient>>,
    app_name: &'static str,
    chat_tool_policy: ChatToolPolicy,
    args: HashMap<String, ArgValue>,
) -> CommandResult {
    runtime::execute_chat(
        ctx,
        registry_fallback,
        risk_policy,
        ailoop_client,
        app_name,
        chat_tool_policy,
        args,
    )
    .await
}
