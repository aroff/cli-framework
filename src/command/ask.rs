//! Ask command implementation
//!
//! Provides natural language command resolution using LLM providers.
//! Users can type natural language queries that get resolved to structured commands.

use crate::ailoop::AiloopContext;
use crate::app::context::AppContext;
use crate::command::{Command, CommandArgs, CommandResult};
use crate::llm::{CommandMetadata, CommandResolution, LlmProvider};
use anyhow::Result;
use std::sync::Arc;

/// Ask command for natural language command resolution
pub fn create_ask_command(llm_provider: Arc<dyn LlmProvider>) -> Command {
    Command {
        id: "ask",
        summary: "Resolve natural language queries to commands",
        syntax: Some("ask <query>"),
        category: Some("ai"),
        execute: |ctx, args| {
            // Note: This is a simplified implementation. In practice, the LLM provider
            // should be stored in the app context or passed differently
            Box::pin(async move {
                // For now, return an error indicating AI is not available
                // In a real implementation, this would access the LLM provider from context
                println!("❌ AI ask command not properly configured in this example");
                println!("   To use AI features, configure LLM provider in AppBuilder");
                Ok(())
            })
        },
    }
}

/// Execute the ask command
async fn execute_ask(
    ctx: &mut dyn AppContext,
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

    // Get available commands (placeholder implementation)
    let available_commands = get_available_commands_placeholder();

    println!("🤔 Thinking about: \"{}\"", query);

    // Get command metadata (placeholder - in practice, this would come from the app)
    let available_commands = get_available_commands_placeholder();

    if available_commands.is_empty() {
        println!("❌ No commands available for resolution");
        return Ok(());
    }

    // Resolve the query using LLM
    let resolution = match llm_provider.resolve_command(&query, &available_commands).await {
        Ok(resolution) => resolution,
        Err(e) => {
            println!("❌ Failed to resolve query: {}", e);
            return Ok(());
        }
    };

    // Display resolution using enhanced CLI output
    crate::cli_output::display_resolution(&resolution);

    // For now, always ask via CLI (ailoop integration needs proper trait design)
    crate::cli_output::display_confirmation(&resolution, None);
    if !request_cli_confirmation(&resolution) {
        println!("❌ Command cancelled");
        return Ok(());
    }

    // Execute the resolved command
    let max_retries = 3;
    let mut attempt = 0;

    while attempt < max_retries {
        if attempt > 0 {
            crate::cli_output::display_retry(attempt + 1, max_retries, &format!("Previous attempt failed"));
        }

        match execute_resolved_command(ctx, &resolution).await {
            Ok(()) => {
                crate::cli_output::display_success(&resolution.command_id);
                return Ok(());
            }
            Err(e) => {
                let error_msg = e.to_string();
                crate::cli_output::display_failure(&resolution.command_id, &error_msg);
                attempt += 1;

                if attempt >= max_retries {
                    crate::cli_output::display_max_retries_exceeded(&resolution.command_id);
                    return Err(e);
                }

                // Ask LLM for retry advice
                if let Some(retry_resolution) = suggest_retry(llm_provider, &query, &error_msg, &available_commands).await {
                    crate::cli_output::display_suggestion(&retry_resolution);
                    // Update resolution with retry suggestion
                    // Note: In practice, this would need more sophisticated logic
                }

                // Wait before retry
                tokio::time::sleep(tokio::time::Duration::from_secs(2)).await;
            }
        }
    }

    Ok(())
}

/// Get available commands (placeholder implementation)
/// In practice, this would access the app's command registry
fn get_available_commands_placeholder() -> Vec<CommandMetadata> {
    vec![
        CommandMetadata {
            id: "deploy".to_string(),
            summary: "Deploy application to environment".to_string(),
            syntax: Some("deploy --env <env> --version <version>".to_string()),
            category: Some("deployment".to_string()),
        },
        CommandMetadata {
            id: "status".to_string(),
            summary: "Show system status".to_string(),
            syntax: Some("status".to_string()),
            category: Some("monitoring".to_string()),
        },
        CommandMetadata {
            id: "logs".to_string(),
            summary: "Show application logs".to_string(),
            syntax: Some("logs --service <service> --lines <count>".to_string()),
            category: Some("monitoring".to_string()),
        },
    ]
}

/// Display command resolution to user
fn display_resolution(resolution: &CommandResolution) {
    println!("\n🎯 Resolved to command:");
    println!("   Command: {}", resolution.command_id);
    println!("   Confidence: {:.1}%", resolution.confidence * 100.0);

    if !resolution.args.positional.is_empty() {
        println!("   Positional args: {:?}", resolution.args.positional);
    }

    if !resolution.args.named.is_empty() {
        println!("   Named args: {:?}", resolution.args.named);
    }

    if let Some(reasoning) = &resolution.reasoning {
        println!("   Reasoning: {}", reasoning);
    }
    println!();
}

/// Request confirmation via CLI input
fn request_cli_confirmation(resolution: &CommandResolution) -> bool {
    println!("Execute this command? (y/N): ");

    let mut input = String::new();
    match std::io::stdin().read_line(&mut input) {
        Ok(_) => {
            let input = input.trim().to_lowercase();
            matches!(input.as_str(), "y" | "yes")
        }
        Err(_) => false,
    }
}

/// Execute the resolved command
async fn execute_resolved_command(
    ctx: &mut dyn AppContext,
    resolution: &CommandResolution,
) -> Result<()> {
    // This is a placeholder - in practice, this would look up the command
    // in the registry and execute it with the resolved arguments

    println!("🔧 Executing command: {} with args {:?}", resolution.command_id, resolution.args);

    // Simulate command execution
    match resolution.command_id.as_str() {
        "deploy" => {
            let env = resolution.args.named.get("env").map(|s| s.as_str()).unwrap_or("dev");
            println!("🚀 Deploying to {} environment...", env);
            // Simulate deployment
            tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;
            println!("✅ Deployment completed");
        }
        "status" => {
            println!("📊 System Status:");
            println!("   Services: OK");
            println!("   Database: OK");
            println!("   Network: OK");
        }
        "logs" => {
            let service = resolution.args.named.get("service").map(|s| s.as_str()).unwrap_or("app");
            let lines_str = resolution.args.named.get("lines").map(|s| s.as_str()).unwrap_or("10");
            let lines = lines_str.parse().unwrap_or(10);
            println!("📋 Last {} lines of {} logs:", lines, service);
            for i in 1..=lines {
                println!("   Line {}: Log message {}", i, i);
            }
        }
        _ => {
            return Err(anyhow::anyhow!("Unknown command: {}", resolution.command_id));
        }
    }

    Ok(())
}

/// Suggest a retry using LLM when command fails
async fn suggest_retry(
    llm_provider: &dyn LlmProvider,
    original_query: &str,
    error_message: &str,
    available_commands: &[CommandMetadata],
) -> Option<CommandResolution> {
    let retry_query = format!(
        "The command failed with error: {}. Original request: {}",
        error_message, original_query
    );

    match llm_provider.resolve_command(&retry_query, available_commands).await {
        Ok(resolution) if resolution.confidence > 0.5 => {
            Some(resolution)
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::llm::{CommandMetadata, CommandResolution};

    // Mock LLM provider for testing
    struct MockLlmProvider;

    #[async_trait::async_trait]
    impl crate::llm::LlmProvider for MockLlmProvider {
        async fn resolve_command(
            &self,
            query: &str,
            _available_commands: &[crate::llm::CommandMetadata],
        ) -> anyhow::Result<crate::llm::CommandResolution> {
            Ok(match query {
                "deploy to production" => crate::llm::CommandResolution {
                    command_id: "deploy".to_string(),
                    args: CommandArgs {
                        positional: vec![],
                        named: [("env".to_string(), "production".to_string())].into_iter().collect(),
                    },
                    confidence: 0.95,
                    reasoning: Some("User wants to deploy to production".to_string()),
                },
                _ => crate::llm::CommandResolution {
                    command_id: "status".to_string(),
                    args: CommandArgs::default(),
                    confidence: 0.8,
                    reasoning: Some("Default fallback".to_string()),
                },
            })
        }
    }

    #[test]
    fn test_create_ask_command() {
        let provider = Arc::new(MockLlmProvider);
        let command = create_ask_command(provider);

        assert_eq!(command.id, "ask");
        assert_eq!(command.summary, "Resolve natural language queries to commands");
        assert!(command.syntax.is_some());
        assert_eq!(command.category, Some("ai"));
    }

    #[test]
    fn test_display_resolution() {
        let resolution = CommandResolution {
            command_id: "deploy".to_string(),
            args: CommandArgs {
                positional: vec!["app".to_string()],
                named: [("env".to_string(), "prod".to_string())].into_iter().collect(),
            },
            confidence: 0.9,
            reasoning: Some("User wants to deploy the app".to_string()),
        };

        // This would normally print to stdout, but we can't easily test that
        display_resolution(&resolution);
    }
}