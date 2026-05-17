use crate::ailoop::AiloopClient;
use crate::app::context::AppContext;
use crate::command::{Command, CommandArgs, CommandRegistry, CommandResult};
use crate::security::command_risk::CommandRiskPolicy;
use crate::spec::arg_spec::{ArgKind, ArgSpec, ArgValueType, Cardinality};
use crate::spec::command_tree::{CommandSpec, EnvVarEntry, ExitCodeEntry};
use async_trait::async_trait;
use serde_json::Value;
use std::collections::HashMap;
use std::sync::Arc;

mod runtime;

pub const CHAT_FEATURE_DISABLED: &str = "CHAT_FEATURE_DISABLED";
pub const CHAT_AGENT_START_FAILED: &str = "CHAT_AGENT_START_FAILED";
pub const CHAT_TOOL_NOT_FOUND: &str = "CHAT_TOOL_NOT_FOUND";
pub const CHAT_ARG_VALIDATION_FAILED: &str = "CHAT_ARG_VALIDATION_FAILED";
pub const CHAT_COMMAND_EXECUTION_FAILED: &str = "CHAT_COMMAND_EXECUTION_FAILED";
pub const CHAT_RISK_REQUIRES_CONFIRMATION: &str = "CHAT_RISK_REQUIRES_CONFIRMATION";
pub const CHAT_DESTRUCTIVE_BLOCKED: &str = "CHAT_DESTRUCTIVE_BLOCKED";
pub const CHAT_TOOL_REGISTRY_COLLISION: &str = "CHAT_TOOL_REGISTRY_COLLISION";

#[async_trait]
pub trait HostToolExecutor: Send + Sync {
    fn list_tools(&self) -> Vec<crate::mcp::schema::McpToolDescriptor>;

    async fn call_tool(
        &self,
        tool_name: &str,
        arguments: Value,
        ctx: &mut dyn AppContext,
        opts: &ChatToolCallOptions,
    ) -> anyhow::Result<()>;
}

#[derive(Debug, Clone)]
pub struct ChatToolCallOptions {
    pub yolo: bool,
    pub interactive: bool,
    pub ailoop_client: Option<Arc<AiloopClient>>,
}

#[derive(Clone)]
pub struct CommandsAsToolsExecutor {
    tools: HashMap<String, Command>,
    app_name: String,
    risk_policy: CommandRiskPolicy,
}

impl CommandsAsToolsExecutor {
    pub fn new(
        registry: &CommandRegistry,
        app_name: &str,
        risk_policy: CommandRiskPolicy,
    ) -> anyhow::Result<Self> {
        if app_name == "unknown" {
            log::warn!("chat: app_name is 'unknown'; use with_version() to set a proper name");
        }
        let mut tools = HashMap::new();
        for (path_str, cmd) in registry.all_tree_commands() {
            if cmd.spec.is_none() {
                log::warn!(
                    "chat: command '{}' has no CommandSpec; using permissive schema",
                    cmd.id
                );
            }
            let tool_name = format!("{}.{}", app_name, path_str.replace('/', "."));
            if tools.insert(tool_name.clone(), cmd.clone()).is_some() {
                return Err(anyhow::anyhow!(
                    "{}: tool name collision for '{}'",
                    CHAT_TOOL_REGISTRY_COLLISION,
                    tool_name
                ));
            }
        }
        Ok(Self {
            tools,
            app_name: app_name.to_string(),
            risk_policy,
        })
    }

    pub fn tool_count(&self) -> usize {
        self.tools.len()
    }

    pub fn app_name(&self) -> &str {
        &self.app_name
    }
}

#[async_trait]
impl HostToolExecutor for CommandsAsToolsExecutor {
    fn list_tools(&self) -> Vec<crate::mcp::schema::McpToolDescriptor> {
        let bridge =
            crate::command_surface::tool_bridge::CommandAsToolBridge::new(self.risk_policy.clone());
        self.tools
            .iter()
            .map(|(name, cmd)| bridge.describe(name, cmd))
            .collect()
    }

    async fn call_tool(
        &self,
        tool_name: &str,
        arguments: Value,
        ctx: &mut dyn AppContext,
        opts: &ChatToolCallOptions,
    ) -> anyhow::Result<()> {
        let cmd = self.tools.get(tool_name).ok_or_else(|| {
            anyhow::anyhow!(
                "{}: tool '{}' not registered",
                CHAT_TOOL_NOT_FOUND,
                tool_name
            )
        })?;

        let confirmation = if opts.yolo {
            crate::command_surface::tool_bridge::ConfirmationMode::AssumeYes
        } else if let Some(ref client) = opts.ailoop_client {
            crate::command_surface::tool_bridge::ConfirmationMode::Ailoop(Arc::clone(client))
        } else if opts.interactive {
            crate::command_surface::tool_bridge::ConfirmationMode::InteractiveStdin
        } else {
            crate::command_surface::tool_bridge::ConfirmationMode::NonInteractive
        };

        let bridge =
            crate::command_surface::tool_bridge::CommandAsToolBridge::new(self.risk_policy.clone());
        let res = bridge
            .invoke(
                ctx,
                crate::command_surface::tool_bridge::BridgeInvocation {
                    command: cmd,
                    input: crate::command_surface::tool_bridge::BridgeInput::Json(arguments),
                    confirmation: confirmation.clone(),
                },
            )
            .await;

        match res {
            Ok(()) => Ok(()),
            Err(crate::command_surface::tool_bridge::BridgeError::ArgValidation(msg)) => {
                Err(anyhow::anyhow!("{}: {}", CHAT_ARG_VALIDATION_FAILED, msg))
            }
            Err(
                crate::command_surface::tool_bridge::BridgeError::SensitiveRequiresConfirmation(
                    cmd_id,
                ),
            ) => {
                // Preserve prior chat error strings:
                // - non-interactive w/ no ailoop => "command ... requires confirmation"
                // - interactive/ailoop available => "user declined confirmation ..."
                if opts.interactive || opts.ailoop_client.is_some() {
                    Err(anyhow::anyhow!(
                        "{}: user declined confirmation for '{}'",
                        CHAT_RISK_REQUIRES_CONFIRMATION,
                        cmd_id
                    ))
                } else {
                    Err(anyhow::anyhow!(
                        "{}: command '{}' is sensitive and requires confirmation",
                        CHAT_RISK_REQUIRES_CONFIRMATION,
                        cmd_id
                    ))
                }
            }
            Err(crate::command_surface::tool_bridge::BridgeError::DestructiveBlocked(cmd_id)) => {
                let env_allowed = std::env::var("ALLOW_DESTRUCTIVE_COMMANDS")
                    .map(|v| v == "1" || v == "true")
                    .unwrap_or(false);

                // Preserve prior chat error strings:
                // - env/terminal preflight failures => "... gated by ALLOW_DESTRUCTIVE_COMMANDS ..."
                // - user decline when confirmation was available => "user declined confirmation ..."
                if !env_allowed || (!opts.interactive && opts.ailoop_client.is_none()) {
                    Err(anyhow::anyhow!(
                        "{}: command '{}' is destructive; gated by ALLOW_DESTRUCTIVE_COMMANDS and interactive confirmation",
                        CHAT_DESTRUCTIVE_BLOCKED,
                        cmd_id
                    ))
                } else {
                    Err(anyhow::anyhow!(
                        "{}: user declined confirmation for '{}'",
                        CHAT_DESTRUCTIVE_BLOCKED,
                        cmd_id
                    ))
                }
            }
            Err(crate::command_surface::tool_bridge::BridgeError::Execution(e)) => {
                Err(anyhow::anyhow!("{}: {}", CHAT_COMMAND_EXECUTION_FAILED, e))
            }
            Err(other) => Err(anyhow::anyhow!(
                "{}: {}",
                CHAT_COMMAND_EXECUTION_FAILED,
                other
            )),
        }
    }
}

pub fn create_chat_command(
    registry: Arc<CommandRegistry>,
    risk_policy: CommandRiskPolicy,
    ailoop_client: Option<Arc<AiloopClient>>,
    app_name: &'static str,
) -> Command {
    Command {
        id: "chat",
        summary: "In-process chat session (commands-as-tools)",
        syntax: Some(
            "chat [-p <prompt>] [--stream] [--yolo] [--model <model>] [--session-agents <agents>]",
        ),
        category: Some("ai"),
        spec: Some(Arc::new(chat_spec())),
        validator: None,
        expose_mcp: false,
        execute: Arc::new(move |ctx, args| {
            let client = ailoop_client.clone();
            let registry = Arc::clone(&registry);
            let risk_policy = risk_policy.clone();
            Box::pin(async move {
                execute_chat(ctx, registry, risk_policy, client, app_name, args).await
            })
        }),
    }
}

fn chat_spec() -> CommandSpec {
    CommandSpec {
        summary: "In-process chat session (commands-as-tools)",
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
            },
        ],
        ..Default::default()
    }
}

async fn execute_chat(
    ctx: &mut dyn AppContext,
    registry_fallback: Arc<CommandRegistry>,
    risk_policy: CommandRiskPolicy,
    ailoop_client: Option<Arc<AiloopClient>>,
    app_name: &'static str,
    args: CommandArgs,
) -> CommandResult {
    runtime::execute_chat(
        ctx,
        registry_fallback,
        risk_policy,
        ailoop_client,
        app_name,
        args,
    )
    .await
}
