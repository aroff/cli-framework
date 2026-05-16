use crate::ailoop::AiloopClient;
use crate::app::context::AppContext;
use crate::command::{Command, CommandArgs, CommandRegistry, CommandResult};
use crate::llm::{CommandMetadata, CommandResolution, LlmProvider};
use crate::security::command_risk::CommandRiskPolicy;
use std::sync::Arc;

pub const ASK_DEPRECATED: &str = "ASK_DEPRECATED";

pub fn create_ask_command(
    llm_provider: Arc<dyn LlmProvider>,
    registry: Arc<CommandRegistry>,
    risk_policy: CommandRiskPolicy,
    ailoop_client: Arc<AiloopClient>,
) -> Command {
    Command {
        id: "ask",
        summary: "Resolve natural language queries to commands",
        syntax: Some("ask <query> | ask --query \"<query>\" [--yes]"),
        category: Some("ai"),
        spec: None,
        validator: None,
        expose_mcp: false,
        execute: Arc::new(move |ctx, args| {
            let provider = llm_provider.clone();
            let registry_fallback = registry.clone();
            let policy = risk_policy.clone();
            let client = ailoop_client.clone();
            Box::pin(async move {
                execute_ask(ctx, provider, registry_fallback, args, policy, client).await
            })
        }),
    }
}

async fn execute_ask(
    ctx: &mut dyn AppContext,
    llm_provider: Arc<dyn LlmProvider>,
    registry_fallback: Arc<CommandRegistry>,
    args: CommandArgs,
    risk_policy: CommandRiskPolicy,
    ailoop_client: Arc<AiloopClient>,
) -> CommandResult {
    #[cfg(feature = "chat")]
    eprintln!(
        "{}: `ask` is deprecated; prefer `chat` (build with `--features chat`).",
        ASK_DEPRECATED
    );

    let query = match extract_query(&args) {
        Ok(q) => q,
        Err(_) => {
            println!("No query provided. Usage: ask <query> or ask --query \"<query>\"");
            return Ok(());
        }
    };

    let available_commands: Vec<CommandMetadata> = {
        let registry = ctx.opt_registry().unwrap_or(registry_fallback.as_ref());
        registry
            .collect_metadata()
            .into_iter()
            .filter(|m| m.id != "ask")
            .collect()
    };

    let resolution = llm_provider
        .resolve_command(&query, &available_commands)
        .await
        .map_err(|e| anyhow::anyhow!("LLM resolution failed: {}", e))?;

    validate_resolution(&resolution)?;

    let assume_yes = args.named.get("yes").map(|v| v == "true").unwrap_or(false)
        || std::env::var("ASK_ASSUME_YES")
            .map(|v| v == "1" || v == "true")
            .unwrap_or(false);

    print_resolution(&resolution);

    if assume_yes {
        println!("\u{26a0}\u{fe0f}  Running without confirmation");
    }

    let command = {
        let registry = ctx.opt_registry().unwrap_or(registry_fallback.as_ref());
        registry
            .get(&resolution.command_id)
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "Resolved command '{}' not found in registry",
                    resolution.command_id
                )
            })?
            .clone()
    };

    let bridge = crate::command_surface::tool_bridge::CommandAsToolBridge::new(risk_policy);
    let confirmation = if assume_yes {
        crate::command_surface::tool_bridge::ConfirmationMode::AssumeYes
    } else {
        // Preserve prior ask behavior: confirmations run through ailoop (HITL).
        crate::command_surface::tool_bridge::ConfirmationMode::Ailoop(ailoop_client)
    };

    let res = bridge
        .invoke(
            ctx,
            crate::command_surface::tool_bridge::BridgeInvocation {
                command: &command,
                input: crate::command_surface::tool_bridge::BridgeInput::Args(resolution.args),
                confirmation,
            },
        )
        .await;

    match res {
        Ok(()) => Ok(()),
        Err(crate::command_surface::tool_bridge::BridgeError::SensitiveConfirmationDeclined(
            _cmd_id,
        )) => {
            println!("Command cancelled by user");
            Ok(())
        }
        Err(crate::command_surface::tool_bridge::BridgeError::SensitiveRequiresConfirmation(
            cmd_id,
        )) => Err(anyhow::anyhow!(
            "SENSITIVE_COMMAND_REQUIRES_CONFIRMATION: command '{}' is sensitive \
             and requires interactive confirmation",
            cmd_id
        )),
        Err(crate::command_surface::tool_bridge::BridgeError::DestructiveConfirmationDeclined(
            _cmd_id,
        )) => {
            println!("Command cancelled by user");
            Ok(())
        }
        Err(crate::command_surface::tool_bridge::BridgeError::DestructiveBlocked(cmd_id)) => {
            Err(anyhow::anyhow!(
                "DESTRUCTIVE_COMMAND_BLOCKED: command '{}' is destructive; \
                 set ALLOW_DESTRUCTIVE_COMMANDS=1 and confirm interactively",
                cmd_id
            ))
        }
        Err(crate::command_surface::tool_bridge::BridgeError::Execution(e)) => Err(e),
        Err(other) => Err(anyhow::anyhow!("{}", other)),
    }
}

fn extract_query(args: &CommandArgs) -> anyhow::Result<String> {
    if !args.positional.is_empty() {
        Ok(args.positional.join(" "))
    } else if let Some(query) = args.named.get("query") {
        Ok(query.clone())
    } else {
        Err(anyhow::anyhow!("No query provided"))
    }
}

fn validate_resolution(resolution: &CommandResolution) -> anyhow::Result<()> {
    if resolution.command_id == "ask" {
        return Err(anyhow::anyhow!("Recursive ask invocation is not allowed"));
    }
    Ok(())
}

fn print_resolution(resolution: &CommandResolution) {
    use crate::security::sanitize_untrusted_output;
    let safe_id = sanitize_untrusted_output(&resolution.command_id);
    println!("\n\u{1f3af} Resolved to command:");
    println!("   Command: {}", safe_id);
    println!("   Confidence: {:.1}%", resolution.confidence * 100.0);
    if let Some(reasoning) = &resolution.reasoning {
        let safe_reasoning = sanitize_untrusted_output(reasoning);
        println!("   Reasoning: {}", safe_reasoning);
    }
    println!();
}

// `dispatch_resolved_command` removed in favor of `CommandAsToolBridge`.
