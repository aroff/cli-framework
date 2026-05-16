use crate::ailoop::AiloopClient;
use crate::app::context::AppContext;
use crate::command::{Command, CommandArgs, CommandRegistry, CommandResult};
use crate::llm::CommandResolution;
use crate::security::command_risk::{CommandRiskPolicy, CommandRiskTier};
use crate::security::RiskEnforcer;
use crate::spec::arg_spec::{ArgKind, ArgSpec, ArgValueType, Cardinality};
use crate::spec::command_tree::{CommandSpec, EnvVarEntry, ExitCodeEntry};
use anyhow::Context;
use async_trait::async_trait;
use serde_json::Value;
use std::collections::HashMap;
use std::io::Write;
use std::path::PathBuf;
use std::sync::Arc;
use tokio_util::sync::CancellationToken;

use aikit_agent::llm::types::{
    FunctionDefinition, LlmMessage, LlmRequest, MessageToolCall, MessageToolCallFunction,
    ToolChoice, ToolDefinition,
};
use aikit_agent::llm::{stream::parse_sse_body, types::LlmStreamEvent};
use aikit_agent::AgentConfig;

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

        let (cmd_args, typed_args) = crate::mcp::map_mcp_args_to_command_args_from_json(arguments)
            .map_err(|e| anyhow::anyhow!("{}: {}", CHAT_ARG_VALIDATION_FAILED, e))?;

        let typed_args_for_validation =
            (cmd.spec.is_some() || cmd.validator.is_some()).then_some(typed_args);

        if let Some(ref typed) = typed_args_for_validation {
            let diags = crate::app::dispatch::validate_typed_args(cmd, typed)
                .map_err(|e| anyhow::anyhow!("{}: {}", CHAT_ARG_VALIDATION_FAILED, e))?;
            if let Some(first) = diags.first() {
                return Err(anyhow::anyhow!(
                    "{}: {}",
                    CHAT_ARG_VALIDATION_FAILED,
                    first.message
                ));
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

        let effective_args = crate::app::dispatch::effective_args_for_execution(
            cmd_args,
            typed_args_for_validation.as_ref(),
        );
        (cmd.execute)(ctx, effective_args)
            .await
            .map_err(|e| anyhow::anyhow!("{}: {}", CHAT_COMMAND_EXECUTION_FAILED, e))
    }
}

async fn enforce_chat_risk_gate(
    policy: &CommandRiskPolicy,
    cmd: &Command,
    yolo: bool,
    interactive: bool,
    ailoop_client: Option<&Arc<AiloopClient>>,
) -> anyhow::Result<()> {
    // Reuse the same risk-gate semantics as `ask` for non-interactive constraints
    // and destructive environment checks, then layer chat-specific prompting on top.
    let resolution = CommandResolution {
        command_id: cmd.id.to_string(),
        args: CommandArgs::default(),
        confidence: 1.0,
        reasoning: None,
    };
    let ailoop_available = ailoop_client.is_some();
    let enforcer = RiskEnforcer::new(policy.clone());
    if let Err(e) =
        enforcer.enforce_preflight(&resolution.command_id, cmd.category, yolo, ailoop_available)
    {
        let msg = e.to_string();
        if msg.contains("SENSITIVE_COMMAND_REQUIRES_CONFIRMATION") {
            return Err(anyhow::anyhow!(
                "{}: command '{}' is sensitive and requires confirmation",
                CHAT_RISK_REQUIRES_CONFIRMATION,
                cmd.id
            ));
        }
        if msg.contains("DESTRUCTIVE_COMMAND_BLOCKED") {
            return Err(anyhow::anyhow!(
                "{}: command '{}' is destructive; gated by ALLOW_DESTRUCTIVE_COMMANDS and interactive confirmation",
                CHAT_DESTRUCTIVE_BLOCKED,
                cmd.id
            ));
        }
        return Err(e);
    }

    let tier = enforcer.classify(cmd.id, cmd.category);
    match tier {
        CommandRiskTier::Safe => Ok(()),
        CommandRiskTier::Sensitive => {
            if yolo {
                return Ok(());
            }
            let confirmed = if let Some(client) = ailoop_client {
                client
                    .request_confirmation(&format!("Execute command '{}'", cmd.id), None)
                    .await?
            } else if interactive {
                prompt_confirm_blocking(format!("Execute command '{}'? [y/N] ", cmd.id)).await?
            } else {
                return Err(anyhow::anyhow!(
                    "{}: command '{}' is sensitive and requires confirmation",
                    CHAT_RISK_REQUIRES_CONFIRMATION,
                    cmd.id
                ));
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
            let confirmed = if let Some(client) = ailoop_client {
                client
                    .request_confirmation(
                        &format!("Execute DESTRUCTIVE command '{}'", cmd.id),
                        None,
                    )
                    .await?
            } else if interactive {
                prompt_confirm_blocking(format!("Execute DESTRUCTIVE command '{}'? [y/N] ", cmd.id))
                    .await?
            } else {
                return Err(anyhow::anyhow!(
                    "{}: command '{}' is destructive; requires interactive confirmation",
                    CHAT_DESTRUCTIVE_BLOCKED,
                    cmd.id
                ));
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

async fn prompt_confirm_blocking(prompt: String) -> anyhow::Result<bool> {
    // Blocking stdin read must not run on the async runtime (§4.5).
    tokio::task::spawn_blocking(move || -> anyhow::Result<bool> {
        let mut stderr = std::io::stderr();
        write!(stderr, "{}", prompt)?;
        stderr.flush()?;

        let mut line = String::new();
        std::io::stdin()
            .read_line(&mut line)
            .context("failed to read confirmation from stdin")?;
        let s = line.trim().to_ascii_lowercase();
        Ok(matches!(s.as_str(), "y" | "yes"))
    })
    .await
    .context("confirmation prompt task failed")?
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
    // MUST use the same frozen registry snapshot as the running `App<C>` when available (§4.3).
    let registry = ctx.opt_registry().unwrap_or(registry_fallback.as_ref());

    let tool_exec = CommandsAsToolsExecutor::new(registry, app_name, risk_policy).map_err(|e| {
        // Deterministic error code for collision (construction-time).
        if e.to_string().contains(CHAT_TOOL_REGISTRY_COLLISION) {
            anyhow::anyhow!("{}", e)
        } else {
            anyhow::anyhow!("{}: {}", CHAT_AGENT_START_FAILED, e)
        }
    })?;

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
        let cancel = CancellationToken::new();
        tokio::select! {
            _ = tokio::signal::ctrl_c() => {
                cancel.cancel();
                return Ok(());
            }
            res = run_agent_one_shot(
                ctx,
                AgentRunOpts { ailoop_client, tool_exec, yolo, stream, model },
                prompt,
                cancel.clone(),
            ) => {
                res?;
            }
        }
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
        AgentRunOpts {
            ailoop_client,
            tool_exec,
            yolo,
            stream,
            model,
        },
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

struct AgentRunOpts {
    ailoop_client: Option<Arc<AiloopClient>>,
    tool_exec: CommandsAsToolsExecutor,
    yolo: bool,
    stream: bool,
    model: Option<String>,
}

async fn repl_loop(ctx: &mut dyn AppContext, opts: AgentRunOpts) -> CommandResult {
    let AgentRunOpts {
        ailoop_client,
        tool_exec,
        yolo,
        stream,
        model,
    } = opts;
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

        let cancel = CancellationToken::new();
        let turn_fut = run_agent_one_shot(
            ctx,
            AgentRunOpts {
                ailoop_client: ailoop_client.clone(),
                tool_exec: tool_exec.clone(),
                yolo,
                stream,
                model: model.clone(),
            },
            prompt.to_string(),
            cancel.clone(),
        );
        tokio::pin!(turn_fut);

        tokio::select! {
            _ = tokio::signal::ctrl_c() => {
                cancel.cancel();
                let _ = (&mut turn_fut).await;
                eprintln!("\nCtrl+C: turn canceled; exiting");
                return Ok(());
            }
            res = &mut turn_fut => {
                res?;
            }
        }
    }
}

async fn run_agent_one_shot(
    ctx: &mut dyn AppContext,
    opts: AgentRunOpts,
    prompt: String,
    cancel: CancellationToken,
) -> CommandResult {
    let AgentRunOpts {
        ailoop_client,
        tool_exec,
        yolo,
        stream,
        model,
    } = opts;
    let workdir = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let config = AgentConfig::from_env(workdir, stream, model)
        .map_err(|e| anyhow::anyhow!("{}: {}", CHAT_AGENT_START_FAILED, e))?;
    let http = Arc::new(
        reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(config.timeout_secs))
            .connect_timeout(std::time::Duration::from_secs(config.connect_timeout_secs))
            .build()
            .context("failed to build HTTP client")?,
    );

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

        let resp = call_llm(http.clone(), req, cancel.clone()).await?;

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
                let args = parse_tool_arguments_blocking(tc.function.arguments).await;

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

async fn parse_tool_arguments_blocking(arguments: String) -> Value {
    tokio::task::spawn_blocking(move || serde_json::from_str(&arguments).unwrap_or(Value::Null))
        .await
        .unwrap_or(Value::Null)
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

async fn call_llm(
    http: Arc<reqwest::Client>,
    req: LlmRequest,
    cancel: CancellationToken,
) -> anyhow::Result<LlmResponseEnvelope> {
    let url = format!("{}/chat/completions", req.base_url.trim_end_matches('/'));

    let mut body = serde_json::json!({
        "model": req.model,
        "messages": req.messages,
        "tools": req.tools,
        "tool_choice": req.tool_choice,
        "temperature": req.temperature,
        "top_p": req.top_p,
        "max_tokens": req.max_tokens,
        "stream": req.stream,
    });
    if req.stream {
        body["stream_options"] = serde_json::json!({ "include_usage": true });
    }

    let send_fut = http
        .post(&url)
        .header("Authorization", format!("Bearer {}", req.api_key))
        .header("Content-Type", "application/json")
        .json(&body)
        .send();

    let response = tokio::select! {
        _ = cancel.cancelled() => {
            return Err(anyhow::anyhow!("{}: cancelled", CHAT_AGENT_START_FAILED));
        }
        res = send_fut => {
            res.map_err(|e| anyhow::anyhow!("{}: {}", CHAT_AGENT_START_FAILED, e))?
        }
    };

    let status = response.status();
    if !status.is_success() {
        let body_text = response.text().await.unwrap_or_default();
        return Err(anyhow::anyhow!(
            "{}: HTTP {} from {}: {}",
            CHAT_AGENT_START_FAILED,
            status.as_u16(),
            url,
            body_text
        ));
    }

    let body_text = tokio::select! {
        _ = cancel.cancelled() => {
            return Err(anyhow::anyhow!("{}: cancelled", CHAT_AGENT_START_FAILED));
        }
        t = response.text() => {
            t.map_err(|e| anyhow::anyhow!("{}: {}", CHAT_AGENT_START_FAILED, e))?
        }
    };

    if req.stream {
        let events = tokio::task::spawn_blocking(move || parse_sse_body(&body_text))
            .await
            .context("stream parse task failed")?
            .map_err(|e| anyhow::anyhow!("{}: {}", CHAT_AGENT_START_FAILED, e))?;

        let mut content = String::new();
        let mut tool_calls_by_id: std::collections::HashMap<String, (String, String)> =
            std::collections::HashMap::new();
        let mut finish_reason = None;

        for ev in events {
            match ev {
                LlmStreamEvent::TextDelta { content: delta } => content.push_str(&delta),
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
                } => finish_reason = Some(r),
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

        return Ok(LlmResponseEnvelope {
            content: Some(content),
            tool_calls,
            finish_reason,
        });
    }

    #[derive(serde::Deserialize)]
    struct OpenAiToolCallFunction {
        name: String,
        arguments: String,
    }
    #[derive(serde::Deserialize)]
    struct OpenAiToolCall {
        id: String,
        #[serde(rename = "type")]
        call_type: Option<String>,
        function: OpenAiToolCallFunction,
    }
    #[derive(serde::Deserialize)]
    struct OpenAiMessage {
        content: Option<String>,
        tool_calls: Option<Vec<OpenAiToolCall>>,
    }
    #[derive(serde::Deserialize)]
    struct OpenAiChoice {
        message: Option<OpenAiMessage>,
        finish_reason: Option<String>,
    }
    #[derive(serde::Deserialize)]
    struct OpenAiResponse {
        choices: Vec<OpenAiChoice>,
    }

    let resp: OpenAiResponse =
        tokio::task::spawn_blocking(move || serde_json::from_str(&body_text))
            .await
            .context("response parse task failed")?
            .map_err(|e| {
                anyhow::anyhow!(
                    "{}: failed to parse response: {}",
                    CHAT_AGENT_START_FAILED,
                    e
                )
            })?;

    let first = resp.choices.into_iter().next();
    let content = first
        .as_ref()
        .and_then(|c| c.message.as_ref())
        .and_then(|m| m.content.clone());
    let tool_calls = first
        .as_ref()
        .and_then(|c| c.message.as_ref())
        .and_then(|m| m.tool_calls.as_ref())
        .map(|calls| {
            calls
                .iter()
                .map(|tc| aikit_agent::llm::types::ToolCall {
                    id: tc.id.clone(),
                    call_type: tc.call_type.clone(),
                    function: aikit_agent::llm::types::ToolCallFunction {
                        name: tc.function.name.clone(),
                        arguments: tc.function.arguments.clone(),
                    },
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let finish_reason = first.and_then(|c| c.finish_reason);

    Ok(LlmResponseEnvelope {
        content,
        tool_calls,
        finish_reason,
    })
}
