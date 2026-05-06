use crate::ailoop::AiloopClient;
use crate::app::context::AppContext;
use crate::command::{Command, CommandArgs, CommandRegistry, CommandResult};
use crate::llm::{CommandMetadata, CommandResolution, LlmProvider};
use crate::security::command_risk::CommandRiskPolicy;
use std::sync::Arc;

struct NoopContext;
impl AppContext for NoopContext {}

pub fn create_ask_command(
    llm_provider: Arc<dyn LlmProvider>,
    registry: Arc<CommandRegistry>,
    risk_policy: CommandRiskPolicy,
    ailoop_client: Arc<AiloopClient>,
) -> Command {
    let metadata_snapshot: Vec<CommandMetadata> = registry
        .collect_metadata()
        .into_iter()
        .filter(|m| m.id != "ask")
        .collect();

    Command {
        id: "ask",
        summary: "Resolve natural language queries to commands",
        syntax: Some("ask <query> | ask --query \"<query>\" [--yes]"),
        category: Some("ai"),
        spec: None,
        validator: None,
        execute: Arc::new(move |_ctx, args| {
            let provider = llm_provider.clone();
            let metadata = metadata_snapshot.clone();
            let registry = registry.clone();
            let policy = risk_policy.clone();
            let client = ailoop_client.clone();
            Box::pin(async move {
                execute_ask(provider, metadata, registry, args, policy, client).await
            })
        }),
    }
}

async fn execute_ask(
    llm_provider: Arc<dyn LlmProvider>,
    available_commands: Vec<CommandMetadata>,
    registry: Arc<CommandRegistry>,
    args: CommandArgs,
    risk_policy: CommandRiskPolicy,
    ailoop_client: Arc<AiloopClient>,
) -> CommandResult {
    let query = match extract_query(&args) {
        Ok(q) => q,
        Err(_) => {
            println!("No query provided. Usage: ask <query> or ask --query \"<query>\"");
            return Ok(());
        }
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

    let command_category = available_commands
        .iter()
        .find(|m| m.id == resolution.command_id)
        .and_then(|m| m.category.as_deref());

    enforce_risk_gate(
        &risk_policy,
        &resolution,
        command_category,
        assume_yes,
        true,
    )?;

    print_resolution(&resolution);

    if assume_yes {
        println!("\u{26a0}\u{fe0f}  Running without confirmation");
    } else {
        let action = format!("Execute command '{}'", resolution.command_id);
        let confirmed = ailoop_client.request_confirmation(&action, None).await?;
        if !confirmed {
            println!("Command cancelled by user");
            return Ok(());
        }
    }

    dispatch_resolved_command(&registry, &resolution).await
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

pub fn enforce_risk_gate(
    policy: &CommandRiskPolicy,
    resolution: &CommandResolution,
    command_category: Option<&str>,
    assume_yes: bool,
    ailoop_available: bool,
) -> anyhow::Result<()> {
    use crate::security::command_risk::CommandRiskTier;
    let tier = policy.classify(&resolution.command_id, command_category);
    match tier {
        CommandRiskTier::Safe => Ok(()),
        CommandRiskTier::Sensitive => {
            if !ailoop_available && !crate::cli_mode::is_interactive() && !assume_yes {
                log::warn!(
                    "Sensitive command '{}' blocked in non-interactive mode without --yes",
                    resolution.command_id
                );
                return Err(anyhow::anyhow!(
                    "SENSITIVE_COMMAND_REQUIRES_CONFIRMATION: command '{}' is sensitive \
                     and requires interactive confirmation",
                    resolution.command_id
                ));
            }
            Ok(())
        }
        CommandRiskTier::Destructive => {
            let env_allowed = std::env::var("ALLOW_DESTRUCTIVE_COMMANDS")
                .map(|v| v == "1" || v == "true")
                .unwrap_or(false);
            if !env_allowed {
                log::warn!(
                    "Destructive command '{}' blocked: ALLOW_DESTRUCTIVE_COMMANDS not set",
                    resolution.command_id
                );
                return Err(anyhow::anyhow!(
                    "DESTRUCTIVE_COMMAND_BLOCKED: command '{}' is destructive; \
                     set ALLOW_DESTRUCTIVE_COMMANDS=1 and confirm interactively",
                    resolution.command_id
                ));
            }
            if !ailoop_available && !crate::cli_mode::is_interactive() {
                log::warn!(
                    "Destructive command '{}' blocked: non-interactive terminal",
                    resolution.command_id
                );
                return Err(anyhow::anyhow!(
                    "DESTRUCTIVE_COMMAND_BLOCKED: command '{}' requires an interactive \
                     terminal or ailoop when ALLOW_DESTRUCTIVE_COMMANDS=1",
                    resolution.command_id
                ));
            }
            Ok(())
        }
    }
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

async fn dispatch_resolved_command(
    registry: &CommandRegistry,
    resolution: &CommandResolution,
) -> CommandResult {
    let command = registry
        .get(&resolution.command_id)
        .ok_or_else(|| {
            anyhow::anyhow!(
                "Resolved command '{}' not found in registry",
                resolution.command_id
            )
        })?
        .clone();

    let mut ctx = NoopContext;
    (command.execute)(&mut ctx, resolution.args.clone()).await
}
