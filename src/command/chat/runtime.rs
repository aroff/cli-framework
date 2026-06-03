use super::*;
use crate::app::context::AppContext;
use crate::command::chat::host_tool_adapter::McpHostToolAdapter;

use anyhow::anyhow;
use std::io::Write;
use std::path::PathBuf;
use std::sync::Arc;
use tokio_util::sync::CancellationToken;

use crate::mcp::{McpToolExportPolicy, McpToolRegistry};

use aikit_agent::llm::openai_compat::OpenAiCompatProvider;
use aikit_agent::{AgentConfig, AgentInternalEvent, Turn};

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

    let tool_exec = Arc::new(
        McpToolRegistry::from_command_registry_with_policy(
            registry,
            app_name,
            McpToolExportPolicy::AllCommands,
        )
        .with_risk_policy(risk_policy),
    );

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
        let workdir = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
        let mut config = AgentConfig::from_env(workdir, stream, model.clone())
            .map_err(|e| anyhow!("{}: {}", CHAT_AGENT_START_FAILED, e))?;

        let adapter = Arc::new(McpHostToolAdapter::new(
            Arc::clone(&tool_exec),
            ChatToolCallOptions {
                yolo,
                interactive: crate::cli_mode::is_interactive(),
                ailoop_client: ailoop_client.clone(),
            },
        ));
        config.host_tool_provider = Some(adapter);

        let gateway = OpenAiCompatProvider::new(config.timeout_secs, config.connect_timeout_secs)
            .map_err(|e| anyhow!("{}: {}", CHAT_AGENT_START_FAILED, e))?;

        let prompt_clone = prompt.clone();
        let run_fut = tokio::task::spawn_blocking(move || {
            aikit_agent::run_with_context(config, vec![], &prompt_clone, Box::new(gateway))
        });

        tokio::select! {
            _ = tokio::signal::ctrl_c() => {
                cancel.cancel();
                return Ok(());
            }
            res = run_fut => {
                let events = res
                    .map_err(|e| anyhow!("{}: task join: {}", CHAT_AGENT_START_FAILED, e))?
                    .map_err(|e| anyhow!("{}: {}", CHAT_AGENT_START_FAILED, e))?;
                print_text_from_events(&events);
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

    repl_loop(AgentRunOpts {
        ailoop_client,
        tool_exec,
        yolo,
        stream,
        model,
    })
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
pub(crate) struct AgentRunOpts {
    pub ailoop_client: Option<Arc<AiloopClient>>,
    pub tool_exec: Arc<McpToolRegistry>,
    pub yolo: bool,
    pub stream: bool,
    pub model: Option<String>,
}

async fn repl_loop(opts: AgentRunOpts) -> CommandResult {
    use tokio::io::{AsyncBufReadExt, BufReader};

    let mut prior_turns: Vec<Turn> = vec![];

    eprintln!("Entering chat REPL. Ctrl+D to exit. Ctrl+C cancels the current turn and exits.");
    let mut reader = BufReader::new(tokio::io::stdin());
    let mut line = String::new();

    loop {
        line.clear();
        eprint!("chat> ");
        let _ = std::io::stderr().flush();

        let n = tokio::select! {
            _ = tokio::signal::ctrl_c() => {
                eprintln!("\nCtrl+C: exiting");
                return Ok(());
            }
            read = reader.read_line(&mut line) => {
                read?
            }
        };

        if n == 0 {
            eprintln!("\nEOF: exiting");
            return Ok(());
        }

        let prompt = line.trim().to_string();
        if prompt.is_empty() {
            continue;
        }

        let workdir = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
        let mut config = AgentConfig::from_env(workdir, opts.stream, opts.model.clone())
            .map_err(|e| anyhow!("{}: {}", CHAT_AGENT_START_FAILED, e))?;

        let adapter = Arc::new(McpHostToolAdapter::new(
            Arc::clone(&opts.tool_exec),
            ChatToolCallOptions {
                yolo: opts.yolo,
                interactive: crate::cli_mode::is_interactive(),
                ailoop_client: opts.ailoop_client.clone(),
            },
        ));
        config.host_tool_provider = Some(adapter);

        let gateway = OpenAiCompatProvider::new(config.timeout_secs, config.connect_timeout_secs)
            .map_err(|e| anyhow!("{}: {}", CHAT_AGENT_START_FAILED, e))?;

        let prior_turns_clone = prior_turns.clone();
        let prompt_clone = prompt.clone();
        let run_fut = tokio::task::spawn_blocking(move || {
            aikit_agent::run_with_context(
                config,
                prior_turns_clone,
                &prompt_clone,
                Box::new(gateway),
            )
        });

        tokio::pin!(run_fut);

        let events = tokio::select! {
            _ = tokio::signal::ctrl_c() => {
                eprintln!("\nCtrl+C: turn canceled; exiting");
                return Ok(());
            }
            res = &mut run_fut => {
                res.map_err(|e| anyhow!("{}: task join: {}", CHAT_AGENT_START_FAILED, e))?
                   .map_err(|e| anyhow!("{}: {}", CHAT_AGENT_START_FAILED, e))?
            }
        };

        prior_turns.extend(turns_from_events(&prompt, &events));
        print_text_from_events(&events);
    }
}

/// Reconstructs conversation history turns from agent events for the next `run_with_context` call.
///
/// Always prepends a user turn for `prompt`. Tool use events are paired with their results;
/// missing results emit a synthetic error turn rather than panicking (AC-MISSING-RESULT).
pub(crate) fn turns_from_events(prompt: &str, events: &[AgentInternalEvent]) -> Vec<Turn> {
    use aikit_agent::context::{ContextToolCall, ContextToolResult};

    let mut turns = Vec::new();
    turns.push(Turn::user(prompt));

    let mut tool_uses: Vec<(String, String, serde_json::Value)> = Vec::new(); // (call_id, name, args)
    let mut tool_results: std::collections::HashMap<String, String> =
        std::collections::HashMap::new();
    let mut text_final: Option<String> = None;

    for event in events {
        match event {
            AgentInternalEvent::ToolUse {
                tool_name,
                tool_input,
                call_id,
            } => {
                tool_uses.push((call_id.clone(), tool_name.clone(), tool_input.clone()));
            }
            AgentInternalEvent::ToolResult {
                call_id, output, ..
            } => {
                tool_results.insert(call_id.clone(), output.clone());
            }
            AgentInternalEvent::TextFinal { content, .. } => {
                text_final = Some(content.clone());
            }
            _ => {}
        }
    }

    if !tool_uses.is_empty() {
        let calls: Vec<ContextToolCall> = tool_uses
            .iter()
            .map(|(call_id, name, args)| ContextToolCall {
                id: call_id.clone(),
                name: name.clone(),
                arguments: args.to_string(),
            })
            .collect();

        let assistant_content = text_final.unwrap_or_default();
        turns.push(Turn::assistant_with_tool_calls(assistant_content, calls));

        for (call_id, _, _) in &tool_uses {
            let output = tool_results
                .get(call_id)
                .cloned()
                .unwrap_or_else(|| format!("ERROR: no result received for tool call {}", call_id));
            turns.push(Turn::tool_result(vec![ContextToolResult {
                call_id: call_id.clone(),
                output,
                is_error: false,
            }]));
        }
    } else if let Some(content) = text_final {
        turns.push(Turn::assistant(content));
    }

    turns
}

fn print_text_from_events(events: &[AgentInternalEvent]) {
    for event in events {
        if let AgentInternalEvent::TextFinal { content, .. } = event {
            let trimmed = content.trim_end();
            if !trimmed.is_empty() {
                println!("{}", trimmed);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::turns_from_events;
    use aikit_agent::AgentInternalEvent;

    /// U8: turns_from_events with TextFinal produces user + assistant turns.
    #[test]
    fn turns_from_events_text_final_produces_assistant_turn() {
        let events = vec![AgentInternalEvent::TextFinal {
            content: "world".to_string(),
            turn_id: None,
        }];
        let turns = turns_from_events("hello", &events);
        assert_eq!(turns.len(), 2);
        assert_eq!(turns[0].content, "hello");
        assert_eq!(turns[1].content, "world");
    }

    /// U9: turns_from_events with ToolUse + ToolResult produces correct turns.
    #[test]
    fn turns_from_events_tool_use_and_result_roundtrip() {
        let events = vec![
            AgentInternalEvent::ToolUse {
                call_id: "c1".to_string(),
                tool_name: "t".to_string(),
                tool_input: serde_json::json!({}),
            },
            AgentInternalEvent::ToolResult {
                call_id: "c1".to_string(),
                output: "out".to_string(),
                is_error: false,
            },
        ];
        let turns = turns_from_events("prompt", &events);
        // user + assistant_with_tool_calls + tool_result
        assert_eq!(turns.len(), 3);
        assert_eq!(turns[1].tool_calls.as_ref().unwrap()[0].id, "c1");
        let result = &turns[2].tool_results.as_ref().unwrap()[0];
        assert_eq!(result.call_id, "c1");
        assert_eq!(result.output, "out");
    }

    /// AC-MISSING-RESULT: ToolUse with no matching ToolResult must not panic.
    #[test]
    fn turns_from_events_missing_tool_result_emits_synthetic_error() {
        let events = vec![AgentInternalEvent::ToolUse {
            call_id: "c1".to_string(),
            tool_name: "t".to_string(),
            tool_input: serde_json::json!({}),
        }];
        let turns = turns_from_events("prompt", &events);
        assert!(turns.len() >= 3);
        let result = &turns[2].tool_results.as_ref().unwrap()[0];
        assert_eq!(result.call_id, "c1");
        assert!(result.output.contains("ERROR"));
    }
}
