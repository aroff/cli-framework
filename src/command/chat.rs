use crate::ailoop::AiloopClient;
use crate::app::context::AppContext;
use crate::command::{Command, CommandArgs, CommandRegistry, CommandResult};
use crate::parser::validator::SpecValidator;
use crate::security::command_risk::{CommandRiskPolicy, CommandRiskTier};
use crate::spec::arg_spec::{ArgKind, ArgSpec, ArgValueType, Cardinality};
use crate::spec::command_tree::{CommandSpec, EnvVarEntry, ExitCodeEntry};
use crate::spec::value::ArgValue;
use anyhow::Context;
use async_trait::async_trait;
use serde_json::Value;
use std::collections::HashMap;
use std::io::Write;
use std::path::PathBuf;
use std::sync::Arc;

use aikit_agent::llm::openai_compat::OpenAiCompatProvider;
use aikit_agent::llm::types::{
    FunctionDefinition, LlmMessage, LlmRequest, MessageToolCall, MessageToolCallFunction,
    ToolChoice, ToolDefinition,
};
use aikit_agent::{AgentConfig, LlmError, LlmGateway};

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
            // Prevent self-recursion and reduce tool confusion.
            if cmd.id == "chat" || cmd.id == "ask" {
                continue;
            }
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

        let (cmd_args, typed_args) = crate::mcp::map_mcp_args_to_command_args_from_json(arguments)
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

        let effective_args = effective_command_args(cmd, cmd_args, &typed_args);

        (cmd.execute)(ctx, effective_args)
            .await
            .map_err(|e| anyhow::anyhow!("{}: {}", CHAT_COMMAND_EXECUTION_FAILED, e))
    }
}

fn effective_command_args(
    cmd: &Command,
    cmd_args: CommandArgs,
    typed_args: &HashMap<String, ArgValue>,
) -> CommandArgs {
    if cmd.spec.is_none() {
        return cmd_args;
    }

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
            if !interactive {
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
    registry: Arc<CommandRegistry>,
    risk_policy: CommandRiskPolicy,
    ailoop_client: Option<Arc<AiloopClient>>,
    app_name: &'static str,
) -> Command {
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
            let client = ailoop_client.clone();
            let tool_exec = tool_executor.clone();
            Box::pin(async move { execute_chat(ctx, client, tool_exec, args).await })
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
    ailoop_client: Option<Arc<AiloopClient>>,
    tool_exec: Option<CommandsAsToolsExecutor>,
    args: CommandArgs,
) -> CommandResult {
    let prompt_flag = args.named.get("prompt").cloned();
    let yolo = args.named.get("yolo").map(|v| v == "true").unwrap_or(false);
    let stream = args
        .named
        .get("stream")
        .map(|v| v == "true")
        .unwrap_or(false);
    let model = args.named.get("model").cloned();

    let prompt_from_stdin = if prompt_flag.is_none() && !crate::cli_mode::is_stdin_tty() {
        Some(read_stdin_all().await?)
    } else {
        None
    };

    if let Some(prompt) = prompt_flag.or(prompt_from_stdin) {
        run_agent_one_shot(ctx, ailoop_client, tool_exec, yolo, stream, model, prompt).await?;
        return Ok(());
    }

    if !crate::cli_mode::is_stdin_tty() {
        return Err(anyhow::anyhow!(
            "{}: no prompt provided and stdin is not a TTY",
            CHAT_AGENT_START_FAILED
        ));
    }

    repl_loop(ctx, ailoop_client, tool_exec, yolo, stream, model).await
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
    ailoop_client: Option<Arc<AiloopClient>>,
    tool_exec: Option<CommandsAsToolsExecutor>,
    yolo: bool,
    stream: bool,
    model: Option<String>,
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

        let turn_fut = run_agent_one_shot(
            ctx,
            ailoop_client.clone(),
            tool_exec.clone(),
            yolo,
            stream,
            model.clone(),
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

async fn run_agent_one_shot(
    ctx: &mut dyn AppContext,
    ailoop_client: Option<Arc<AiloopClient>>,
    tool_exec: Option<CommandsAsToolsExecutor>,
    yolo: bool,
    stream: bool,
    model: Option<String>,
    prompt: String,
) -> CommandResult {
    let tool_exec = tool_exec.ok_or_else(|| {
        anyhow::anyhow!(
            "{}: tool registry unavailable (CHAT_TOOL_REGISTRY_COLLISION?)",
            CHAT_AGENT_START_FAILED
        )
    })?;

    let workdir = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let config = AgentConfig::from_env(workdir, stream, model)
        .map_err(|e| anyhow::anyhow!("{}: {}", CHAT_AGENT_START_FAILED, e))?;

    let gateway = OpenAiCompatProvider::new(config.timeout_secs, config.connect_timeout_secs)
        .map_err(|e| anyhow::anyhow!("{}: {}", CHAT_AGENT_START_FAILED, e))?;
    let gateway = Arc::new(gateway);

    let tools: Vec<ToolDefinition> = tool_exec
        .list_tools()
        .into_iter()
        .map(|t| ToolDefinition {
            tool_type: "function".to_string(),
            function: FunctionDefinition {
                name: t.name,
                description: Some(t.description),
                parameters: t.input_schema,
            },
        })
        .collect();

    let mut messages = vec![LlmMessage {
        role: "system".to_string(),
        content: Some(build_chat_system_instructions()),
        tool_calls: None,
        tool_call_id: None,
    }];
    messages.push(LlmMessage {
        role: "user".to_string(),
        content: Some(prompt),
        tool_calls: None,
        tool_call_id: None,
    });

    let tool_opts = ChatToolCallOptions {
        yolo,
        interactive: crate::cli_mode::is_interactive(),
        ailoop_client,
    };

    for _ in 0..config.max_iterations {
        let req = LlmRequest {
            model: config.model.clone(),
            base_url: config.base_url.clone(),
            api_key: config.api_key.clone(),
            messages: messages.clone(),
            tools: tools.clone(),
            tool_choice: Some(ToolChoice::auto()),
            temperature: None,
            top_p: None,
            max_tokens: None,
            stream,
        };

        let resp = call_llm_off_runtime(Arc::clone(&gateway), req, stream).await?;

        if !resp.tool_calls.is_empty() {
            let tool_calls_for_ctx: Vec<MessageToolCall> = resp
                .tool_calls
                .iter()
                .map(|tc| MessageToolCall {
                    id: tc.id.clone(),
                    call_type: "function".to_string(),
                    function: MessageToolCallFunction {
                        name: tc.function.name.clone(),
                        arguments: tc.function.arguments.clone(),
                    },
                })
                .collect();

            messages.push(LlmMessage {
                role: "assistant".to_string(),
                content: resp.content.clone().filter(|s| !s.is_empty()),
                tool_calls: Some(tool_calls_for_ctx),
                tool_call_id: None,
            });

            for tc in resp.tool_calls {
                let tool_name = tc.function.name;
                let call_id = tc.id;
                let args: Value =
                    serde_json::from_str(&tc.function.arguments).unwrap_or(Value::Null);

                // Tool calls are serialized by design.
                let output = match tool_exec.call_tool(&tool_name, args, ctx, &tool_opts).await {
                    Ok(()) => "OK".to_string(),
                    Err(e) => e.to_string(),
                };

                messages.push(LlmMessage {
                    role: "tool".to_string(),
                    content: Some(output),
                    tool_calls: None,
                    tool_call_id: Some(call_id),
                });
            }

            continue;
        }

        if let Some(text) = resp.content.as_ref() {
            if !text.trim().is_empty() {
                println!("{}", text.trim_end());
            }
        }

        messages.push(LlmMessage {
            role: "assistant".to_string(),
            content: resp.content.clone(),
            tool_calls: None,
            tool_call_id: None,
        });

        if resp.finish_reason.as_deref() == Some("stop") {
            return Ok(());
        }
    }

    Err(anyhow::anyhow!(
        "{}: exceeded max iterations ({})",
        CHAT_AGENT_START_FAILED,
        config.max_iterations
    ))
}

fn build_chat_system_instructions() -> String {
    let mut s = String::new();
    s.push_str("You are an in-process CLI agent.\n");
    s.push_str("You can only use the provided tools, which correspond to this app's registered CLI commands.\n");
    s.push_str("Prefer using tools to perform actions. After completing tool calls, respond to the user with a short summary.\n");
    s
}

struct LlmResponseEnvelope {
    content: Option<String>,
    tool_calls: Vec<aikit_agent::llm::types::ToolCall>,
    finish_reason: Option<String>,
}

async fn call_llm_off_runtime(
    gateway: Arc<OpenAiCompatProvider>,
    req: LlmRequest,
    stream: bool,
) -> anyhow::Result<LlmResponseEnvelope> {
    let handle = tokio::task::spawn_blocking(
        move || -> Result<(Option<String>, Vec<aikit_agent::llm::types::ToolCall>, Option<String>), LlmError> {
        if stream {
            use aikit_agent::llm::types::LlmStreamEvent;
            let mut content = String::new();
            let mut tool_calls_by_id: std::collections::HashMap<String, (String, String)> =
                std::collections::HashMap::new();
            let mut finish_reason = None;

            let stream_handle = gateway.stream(req)?;
            for ev in stream_handle {
                match ev? {
                    LlmStreamEvent::TextDelta { content: delta } => {
                        content.push_str(&delta);
                    }
                    LlmStreamEvent::ToolCallDelta {
                        id,
                        function_name,
                        arguments_delta,
                    } => {
                        let entry = tool_calls_by_id
                            .entry(id)
                            .or_insert_with(|| (function_name.clone(), String::new()));
                        entry.0 = function_name;
                        entry.1.push_str(&arguments_delta);
                    }
                    LlmStreamEvent::Completed {
                        finish_reason: r, ..
                    } => {
                        finish_reason = Some(r);
                    }
                    _ => {}
                }
            }

            let tool_calls = tool_calls_by_id
                .into_iter()
                .map(|(id, (name, args))| aikit_agent::llm::types::ToolCall {
                    id,
                    call_type: None,
                    function: aikit_agent::llm::types::ToolCallFunction {
                        name,
                        arguments: args,
                    },
                })
                .collect();

            Ok((Some(content), tool_calls, finish_reason))
        } else {
            let resp = gateway.complete(req)?;
            Ok((resp.content, resp.tool_calls, resp.finish_reason))
        }
        },
    );

    let (content, tool_calls, finish_reason) =
        handle
            .await
            .context("chat LLM call join failed")?
            .map_err(|e| anyhow::anyhow!("{}: {}", CHAT_AGENT_START_FAILED, e))?;

    Ok(LlmResponseEnvelope {
        content,
        tool_calls,
        finish_reason,
    })
}
