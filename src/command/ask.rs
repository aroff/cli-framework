//! Ask command implementation
//!
//! Provides natural language command resolution using LLM providers.
//! Users can type natural language queries that get resolved to structured commands.

use crate::app::context::AppContext;
use crate::command::{Command, CommandArgs, CommandResult};
use crate::llm::{CommandMetadata, LlmProvider};
use std::sync::Arc;

/// Ask command for natural language command resolution
pub fn create_ask_command(_llm_provider: Arc<dyn LlmProvider>) -> Command {
    Command {
        id: "ask",
        summary: "Resolve natural language queries to commands",
        syntax: Some("ask <query>"),
        category: Some("ai"),
        execute: |_ctx, _args| {
            Box::pin(async move {
                // In a real framework, this would be:
                // let provider = ctx.llm_provider();
                // But we need to be able to call it on the trait.
                
                println!("🤖 AI resolution starting...");
                println!("✅ AI resolution complete (simulated)");
                
                Ok(())
            })
        },
    }
}

/// Execute the ask command
async fn _execute_ask(
    _ctx: &mut dyn AppContext,
    args: CommandArgs,
    llm_provider: &dyn LlmProvider,
) -> CommandResult {
    // Extract the query from arguments
    let query = if !args.positional.is_empty() {
        args.positional.join(" ")
    } else if let Some(query) = args.named.get("query") {
        query.clone()
    } else {
        println!("❌ Error: No query provided. Usage: ask <query> or ask --query \"<query>\"");
        return Ok(());
    };

    println!("🤔 Thinking about: \"{}\"...", query);

    // Placeholder commands
    let available_commands = vec![
        CommandMetadata {
            id: "hello".to_string(),
            summary: "Say hello to someone".to_string(),
            syntax: Some("hello <name>".to_string()),
            category: Some("utilities".to_string()),
        },
    ];

    // Resolve the query using LLM
    let resolution = match llm_provider.resolve_command(&query, &available_commands).await {
        Ok(resolution) => resolution,
        Err(e) => {
            println!("❌ Failed to resolve query: {}", e);
            return Ok(());
        }
    };

    // Display resolution
    println!("\n🎯 Resolved to command:");
    println!("   Command: {}", resolution.command_id);
    println!("   Confidence: {:.1}%", resolution.confidence * 100.0);
    if let Some(reasoning) = &resolution.reasoning {
        println!("   Reasoning: {}", reasoning);
    }
    println!();

    // Request confirmation
    println!("Execute this command? (y/N): ");
    let mut input = String::new();
    std::io::stdin().read_line(&mut input)?;
    let input = input.trim().to_lowercase();
    if !matches!(input.as_str(), "y" | "yes") {
        println!("❌ Command cancelled");
        return Ok(());
    }

    println!("🔧 Executing command: {} with args {:?}", resolution.command_id, resolution.args);
    println!("✅ Command execution simulated");

    Ok(())
}
