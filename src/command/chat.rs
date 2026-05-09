use crate::ailoop::AiloopClient;
use crate::app::context::AppContext;
use crate::command::{Command, CommandArgs, CommandRegistry, CommandResult};
use crate::llm::{CommandMetadata, LlmProvider};
use crate::parser::validator::SpecValidator;
use crate::security::command_risk::{CommandRiskPolicy, CommandRiskTier};
use crate::spec::arg_spec::{ArgKind, ArgSpec, ArgValueType, Cardinality};
use crate::spec::command_tree::{CommandSpec, EnvVarEntry, ExitCodeEntry};
use crate::spec::value::ArgValue;
use crate::tool_args::map_tool_args_to_command_args;
use anyhow::Context;
use async_trait::async_trait;
use serde_json::Value;
use std::collections::HashMap;
use std::io::Write;
use std::sync::Arc;

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
        self.tools
            .iter()
            .map(|(name, cmd)| {
                crate::mcp::schema::command_to_tool_descriptor(
                    name,
                    cmd.summary,
                    cmd.spec.as_deref(),
                )
            })
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

        let (cmd_args, typed_args) = map_tool_args_to_command_args(arguments)
            .map_err(|e| anyhow::anyhow!("{}: {}", CHAT_ARG_VALIDATION_FAILED, e))?;

        if let Some(ref spec) = cmd.spec {
            let diagnostics = SpecValidator::validate(spec, &typed_args);
            if !diagnostics.is_empty() {
                return Err(anyhow::anyhow!(
                    "{}: {}",
                    CHAT_ARG_VALIDATION_FAILED,
                    diagnostics[0].message
                ));
            }
            if let Some(ref validator) = cmd.validator {
                let custom_diags = validator(&typed_args);
                if !custom_diags.is_empty() {
                    return Err(anyhow::anyhow!(
                        "{}: {}",
                        CHAT_ARG_VALIDATION_FAILED,
                        custom_diags[0].message
                    ));
                }
            }
        }

        enforce_chat_risk_gate(
            &self.risk_policy,
            cmd,
            opts.yolo,
            opts.interactive,
            opts.ailoop_client.as_ref(),
        )
        .await?;

        let effective_args = if cmd.spec.is_some() {
            command_args_from_typed_args(&typed_args)
        } else {
            cmd_args
        };

        (cmd.execute)(ctx, effective_args)
            .await
            .map_err(|e| anyhow::anyhow!("{}: {}", CHAT_COMMAND_EXECUTION_FAILED, e))
    }
}

fn command_args_from_typed_args(typed_args: &HashMap<String, ArgValue>) -> CommandArgs {
    let mut named = HashMap::new();
    for (k, v) in typed_args {
        let s = match v {
            ArgValue::Bool(b) => b.to_string(),
            ArgValue::Str(s) => s.clone(),
            ArgValue::Int(i) => i.to_string(),
            ArgValue::Float(f) => f.to_string(),
            ArgValue::Enum(e) => e.clone(),
            ArgValue::Count(c) => c.to_string(),
            ArgValue::List(items) => items
                .iter()
                .map(|i| match i {
                    ArgValue::Str(s) => s.clone(),
                    ArgValue::Int(i) => i.to_string(),
                    ArgValue::Float(f) => f.to_string(),
                    ArgValue::Enum(e) => e.clone(),
                    _ => String::new(),
                })
                .collect::<Vec<_>>()
                .join(","),
        };
        named.insert(k.clone(), s);
    }
    CommandArgs {
        positional: Vec::new(),
        named,
    }
}

async fn enforce_chat_risk_gate(
    policy: &CommandRiskPolicy,
    cmd: &Command,
    yolo: bool,
    interactive: bool,
    ailoop_client: Option<&Arc<AiloopClient>>,
) -> anyhow::Result<()> {
    let tier = policy.classify(cmd.id, cmd.category);
    match tier {
        CommandRiskTier::Safe => Ok(()),
        CommandRiskTier::Sensitive => {
            if yolo {
                return Ok(());
            }
            if !interactive && ailoop_client.is_none() {
                return Err(anyhow::anyhow!(
                    "{}: command '{}' is sensitive and requires confirmation (use --yolo in non-interactive mode)",
                    CHAT_RISK_REQUIRES_CONFIRMATION,
                    cmd.id
                ));
            }
            let confirmed = if let Some(client) = ailoop_client {
                client
                    .request_confirmation(&format!("Execute command '{}'", cmd.id), None)
                    .await?
            } else {
                prompt_confirm(&format!("Execute command '{}'? [y/N] ", cmd.id))?
            };
            if !confirmed {
                return Err(anyhow::anyhow!(
                    "{}: user declined confirmation for '{}'",
                    CHAT_RISK_REQUIRES_CONFIRMATION,
                    cmd.id
                ));
            }
            Ok(())
        }
        CommandRiskTier::Destructive => {
            let env_allowed = std::env::var("ALLOW_DESTRUCTIVE_COMMANDS")
                .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
                .unwrap_or(false);
            if !env_allowed {
                return Err(anyhow::anyhow!(
                    "{}: command '{}' is destructive; set ALLOW_DESTRUCTIVE_COMMANDS=1 and confirm interactively",
                    CHAT_DESTRUCTIVE_BLOCKED,
                    cmd.id
                ));
            }
            if !interactive && ailoop_client.is_none() {
                return Err(anyhow::anyhow!(
                    "{}: command '{}' requires interactive confirmation or ailoop when ALLOW_DESTRUCTIVE_COMMANDS=1",
                    CHAT_DESTRUCTIVE_BLOCKED,
                    cmd.id
                ));
            }
            let confirmed = if let Some(client) = ailoop_client {
                client
                    .request_confirmation(
                        &format!("Execute DESTRUCTIVE command '{}'", cmd.id),
                        None,
                    )
                    .await?
            } else {
                prompt_confirm(&format!("Execute DESTRUCTIVE command '{}'? [y/N] ", cmd.id))?
            };
            if !confirmed {
                return Err(anyhow::anyhow!(
                    "{}: user declined confirmation for '{}'",
                    CHAT_DESTRUCTIVE_BLOCKED,
                    cmd.id
                ));
            }
            Ok(())
        }
    }
}

fn prompt_confirm(prompt: &str) -> anyhow::Result<bool> {
    let mut stderr = std::io::stderr();
    write!(stderr, "{}", prompt)?;
    stderr.flush()?;

    let mut line = String::new();
    std::io::stdin()
        .read_line(&mut line)
        .context("failed to read confirmation from stdin")?;
    let s = line.trim().to_ascii_lowercase();
    Ok(matches!(s.as_str(), "y" | "yes"))
}

pub fn create_chat_command(
    llm_provider: Option<Arc<dyn LlmProvider>>,
    registry: Arc<CommandRegistry>,
    risk_policy: CommandRiskPolicy,
    ailoop_client: Option<Arc<AiloopClient>>,
    app_name: &'static str,
) -> Command {
    let metadata_snapshot: Vec<CommandMetadata> = registry
        .collect_metadata()
        .into_iter()
        .filter(|m| m.id != "chat" && m.id != "ask")
        .collect();

    let tool_executor = CommandsAsToolsExecutor::new(&registry, app_name, risk_policy.clone())
        .map_err(|e| {
            log::error!("chat tool registry construction failed: {}", e);
            e
        })
        .ok();

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
            let provider = llm_provider.clone();
            let registry = registry.clone();
            let policy = risk_policy.clone();
            let client = ailoop_client.clone();
            let metadata = metadata_snapshot.clone();
            let tool_exec = tool_executor.clone();
            Box::pin(async move {
                execute_chat(
                    ctx, provider, registry, metadata, policy, client, tool_exec, args,
                )
                .await
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
 - This rollout phase uses the configured LLM provider to resolve one command per turn.\n\
 - Structured output modes (events/JSON) require upstream library support and are not enabled here.",
        ),
        env_vars: vec![
            EnvVarEntry {
                name: "ALLOW_DESTRUCTIVE_COMMANDS",
                description: "Set to 1/true to allow destructive commands (still requires confirmation)",
            },
            EnvVarEntry {
                name: "ASK_ASSUME_YES",
                description: "Legacy ask env var (ignored by chat; use --yolo)",
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
                help: "Model override (provider-dependent; best-effort)",
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
                help: "Skip confirmations for Sensitive commands (does not bypass Destructive gating)",
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
                help: "Stream LLM output if supported by the provider",
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
                help: "Session agent selection (reserved; provider-dependent)",
            },
        ],
        ..Default::default()
    }
}

async fn execute_chat(
    ctx: &mut dyn AppContext,
    llm_provider: Option<Arc<dyn LlmProvider>>,
    registry: Arc<CommandRegistry>,
    available_commands: Vec<CommandMetadata>,
    risk_policy: CommandRiskPolicy,
    ailoop_client: Option<Arc<AiloopClient>>,
    tool_exec: Option<CommandsAsToolsExecutor>,
    args: CommandArgs,
) -> CommandResult {
    let prompt_flag = args.named.get("prompt").cloned();
    let yolo = args.named.get("yolo").map(|v| v == "true").unwrap_or(false);

    let prompt_from_stdin = if prompt_flag.is_none() && !crate::cli_mode::is_stdin_tty() {
        Some(read_stdin_all().await?)
    } else {
        None
    };

    if let Some(prompt) = prompt_flag.or(prompt_from_stdin) {
        run_single_turn(
            ctx,
            llm_provider,
            registry,
            available_commands,
            risk_policy,
            ailoop_client,
            tool_exec,
            yolo,
            prompt,
        )
        .await?;
        return Ok(());
    }

    if !crate::cli_mode::is_stdin_tty() {
        return Err(anyhow::anyhow!(
            "{}: no prompt provided and stdin is not a TTY",
            CHAT_AGENT_START_FAILED
        ));
    }

    repl_loop(
        ctx,
        llm_provider,
        registry,
        available_commands,
        risk_policy,
        ailoop_client,
        tool_exec,
        yolo,
    )
    .await
}

async fn read_stdin_all() -> anyhow::Result<String> {
    use tokio::io::AsyncReadExt;
    let mut buf = Vec::new();
    let mut stdin = tokio::io::stdin();
    stdin.read_to_end(&mut buf).await?;
    Ok(String::from_utf8_lossy(&buf).trim().to_string())
}

async fn repl_loop(
    ctx: &mut dyn AppContext,
    llm_provider: Option<Arc<dyn LlmProvider>>,
    registry: Arc<CommandRegistry>,
    available_commands: Vec<CommandMetadata>,
    risk_policy: CommandRiskPolicy,
    ailoop_client: Option<Arc<AiloopClient>>,
    tool_exec: Option<CommandsAsToolsExecutor>,
    yolo: bool,
) -> CommandResult {
    use tokio::io::{AsyncBufReadExt, BufReader};

    eprintln!("Entering chat REPL. Ctrl+D to exit. Ctrl+C cancels the current turn and exits.");
    let mut reader = BufReader::new(tokio::io::stdin());
    let mut line = String::new();

    loop {
        line.clear();
        eprint!("chat> ");
        let _ = std::io::stderr().flush();

        tokio::select! {
            _ = tokio::signal::ctrl_c() => {
                eprintln!("\nCtrl+C: exiting");
                return Ok(());
            }
            read = reader.read_line(&mut line) => {
                let n = read?;
                if n == 0 {
                    eprintln!("\nEOF: exiting");
                    return Ok(());
                }
            }
        }

        let prompt = line.trim();
        if prompt.is_empty() {
            continue;
        }

        let turn_fut = run_single_turn(
            ctx,
            llm_provider.clone(),
            Arc::clone(&registry),
            available_commands.clone(),
            risk_policy.clone(),
            ailoop_client.clone(),
            tool_exec.clone(),
            yolo,
            prompt.to_string(),
        );

        tokio::select! {
            _ = tokio::signal::ctrl_c() => {
                eprintln!("\nCtrl+C: turn canceled; exiting");
                return Ok(());
            }
            res = turn_fut => {
                res?;
            }
        }
    }
}

async fn run_single_turn(
    ctx: &mut dyn AppContext,
    llm_provider: Option<Arc<dyn LlmProvider>>,
    registry: Arc<CommandRegistry>,
    available_commands: Vec<CommandMetadata>,
    risk_policy: CommandRiskPolicy,
    ailoop_client: Option<Arc<AiloopClient>>,
    tool_exec: Option<CommandsAsToolsExecutor>,
    yolo: bool,
    prompt: String,
) -> CommandResult {
    let provider = llm_provider.ok_or_else(|| {
        anyhow::anyhow!(
            "{}: no LLM provider configured; call AppBuilder::with_llm_provider()",
            CHAT_AGENT_START_FAILED
        )
    })?;

    // Run resolution off the runtime (keeps the tokio executor responsive even if the provider
    // implementation does blocking work internally).
    let resolution = tokio::task::spawn_blocking({
        let provider = provider.clone();
        let prompt = prompt.clone();
        let available = available_commands.clone();
        move || {
            tokio::runtime::Handle::current()
                .block_on(async move { provider.resolve_command(&prompt, &available).await })
        }
    })
    .await
    .context("chat turn task join failed")?
    .map_err(|e| anyhow::anyhow!("{}: {}", CHAT_AGENT_START_FAILED, e))?;

    // Risk gate semantics for the resolved command mirror ask.
    let command_category = available_commands
        .iter()
        .find(|m| m.id == resolution.command_id)
        .and_then(|m| m.category.as_deref());

    crate::command::ask::enforce_risk_gate(
        &risk_policy,
        &resolution,
        command_category,
        yolo,
        ailoop_client.is_some(),
    )?;

    // Execute under the *real* ctx (no NoopContext).
    let cmd = registry
        .get(&resolution.command_id)
        .ok_or_else(|| anyhow::anyhow!("Resolved command '{}' not found", resolution.command_id))?
        .clone();

    if let Some(exec) = tool_exec {
        // Best-effort: if the tool name exists (root command ID), execute via tool path.
        // This does not implement full multi-tool planning yet, but it exercises the same
        // mapping/validation+execution code path as agent tool calls.
        let tool_name = format!("{}.{}", exec.app_name(), resolution.command_id);
        let mut args_obj = serde_json::Map::new();
        if !resolution.args.positional.is_empty() {
            args_obj.insert(
                "_positional".to_string(),
                Value::Array(
                    resolution
                        .args
                        .positional
                        .iter()
                        .map(|s| Value::String(s.clone()))
                        .collect(),
                ),
            );
        }
        for (k, v) in &resolution.args.named {
            args_obj.insert(k.clone(), Value::String(v.clone()));
        }

        let opts = ChatToolCallOptions {
            yolo,
            interactive: crate::cli_mode::is_interactive(),
            ailoop_client,
        };

        if exec.tools.contains_key(&tool_name) {
            return exec
                .call_tool(&tool_name, Value::Object(args_obj), ctx, &opts)
                .await;
        }
    }

    (cmd.execute)(ctx, resolution.args).await
}
