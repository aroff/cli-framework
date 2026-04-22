use crate::app::context::AppContext;
use crate::command::{Command, CommandArgs, CommandRegistry, CommandResult};
use crate::llm::{CommandMetadata, CommandResolution, LlmProvider};
use std::sync::Arc;

struct NoopContext;
impl AppContext for NoopContext {}

pub fn create_ask_command(
    llm_provider: Arc<dyn LlmProvider>,
    registry: Arc<CommandRegistry>,
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
        execute: Arc::new(move |_ctx, args| {
            let provider = llm_provider.clone();
            let metadata = metadata_snapshot.clone();
            let registry = registry.clone();
            Box::pin(async move { execute_ask(provider, metadata, registry, args).await })
        }),
    }
}

async fn execute_ask(
    llm_provider: Arc<dyn LlmProvider>,
    available_commands: Vec<CommandMetadata>,
    registry: Arc<CommandRegistry>,
    args: CommandArgs,
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

    print_resolution(&resolution);

    let assume_yes = args.named.get("yes").map(|v| v == "true").unwrap_or(false)
        || std::env::var("ASK_ASSUME_YES")
            .map(|v| v == "1" || v == "true")
            .unwrap_or(false);

    if assume_yes {
        println!("\u{26a0}\u{fe0f}  Running without confirmation");
    } else if !confirm_execution()? {
        println!("Command cancelled");
        return Ok(());
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

fn print_resolution(resolution: &CommandResolution) {
    println!("\n\u{1f3af} Resolved to command:");
    println!("   Command: {}", resolution.command_id);
    println!("   Confidence: {:.1}%", resolution.confidence * 100.0);
    if let Some(reasoning) = &resolution.reasoning {
        println!("   Reasoning: {}", reasoning);
    }
    println!();
}

fn confirm_execution() -> anyhow::Result<bool> {
    println!("Execute this command? (y/N): ");
    let mut input = String::new();
    std::io::stdin().read_line(&mut input)?;
    let input = input.trim().to_lowercase();
    Ok(matches!(input.as_str(), "y" | "yes"))
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
