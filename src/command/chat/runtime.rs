use super::*;

use anyhow::Context;
use std::io::Write;
use std::path::PathBuf;
use std::sync::Arc;
use tokio_util::sync::CancellationToken;

use aikit_agent::llm::stream::parse_sse_body;
use aikit_agent::llm::types::LlmStreamEvent;
use aikit_agent::llm::types::{
    FunctionDefinition, LlmMessage, LlmRequest, MessageToolCall, MessageToolCallFunction,
    ToolChoice, ToolDefinition,
};
use aikit_agent::AgentConfig;

pub(super) async fn execute_chat(
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

#[derive(Clone)]
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
    s.push_str(
        "You can only use the provided tools, which correspond to this app's registered CLI commands.\n",
    );
    s.push_str(
        "Prefer using tools to perform actions. After completing tool calls, respond to the user with a short summary.\n",
    );
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
