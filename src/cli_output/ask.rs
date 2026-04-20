//! CLI output utilities for ask command
//!
//! Provides specialized formatting for displaying command resolution results,
//! confidence scores, retry attempts, and error messages.

use crate::llm::CommandResolution;

/// Display command resolution result
///
/// Shows the resolved command, confidence score, arguments, and reasoning.
pub fn display_resolution(resolution: &CommandResolution) {
    println!("\n🤔 Resolving command...");

    // Show confidence with color coding
    let confidence_color = get_confidence_color(resolution.confidence);
    println!(
        "🎯 Resolved: {} (confidence: {}{:.1}%{})",
        resolution.command_id,
        confidence_color,
        resolution.confidence * 100.0,
        reset_color()
    );

    // Show arguments if any
    if !resolution.args.positional.is_empty() || !resolution.args.named.is_empty() {
        println!("📋 Arguments:");
        if !resolution.args.positional.is_empty() {
            println!("   Positional: {:?}", resolution.args.positional);
        }
        if !resolution.args.named.is_empty() {
            println!("   Named: {:?}", resolution.args.named);
        }
    }

    // Show reasoning if available
    if let Some(reasoning) = &resolution.reasoning {
        println!("💭 Reasoning: {}", reasoning);
    }

    println!();
}

/// Display confirmation prompt
///
/// Shows a formatted confirmation prompt with command details.
pub fn display_confirmation(resolution: &CommandResolution, context: Option<&str>) {
    println!("⚠️  Execute command: {}", resolution.command_id);

    if !resolution.args.positional.is_empty() {
        println!("   Positional args: {:?}", resolution.args.positional);
    }

    if !resolution.args.named.is_empty() {
        for (key, value) in &resolution.args.named {
            println!("   {}: {}", key, value);
        }
    }

    if let Some(ctx) = context {
        println!("   Context: {}", ctx);
    }

    println!();
}

/// Display retry information
///
/// Shows information about retry attempts with error details.
pub fn display_retry(attempt: usize, max_attempts: usize, error: &str) {
    let attempt_color = if attempt >= max_attempts {
        red_color()
    } else {
        yellow_color()
    };

    println!(
        "{}🔄 Retry {} of {}{}",
        attempt_color,
        attempt,
        max_attempts,
        reset_color()
    );

    println!("❌ Previous attempt failed: {}", error);
    println!();
}

/// Display successful command execution
///
/// Shows confirmation that the command executed successfully.
pub fn display_success(command_id: &str) {
    println!("✅ {} executed successfully", command_id);
}

/// Display command execution failure
///
/// Shows detailed error information for failed commands.
pub fn display_failure(command_id: &str, error: &str) {
    println!("❌ {} failed: {}", command_id, error);
}

/// Display max retries exceeded
///
/// Shows when all retry attempts have been exhausted.
pub fn display_max_retries_exceeded(command_id: &str) {
    println!(
        "{}💥 Max retries exceeded for command: {}{}",
        red_color(),
        command_id,
        reset_color()
    );
}

/// Display suggested alternative command
///
/// Shows AI-suggested alternative when a command fails.
pub fn display_suggestion(resolution: &CommandResolution) {
    println!("💡 AI suggests trying:");
    display_resolution(resolution);
}

/// Display command help for available commands
///
/// Shows a formatted list of available commands for user reference.
pub fn display_command_help(commands: &[crate::llm::CommandMetadata]) {
    if commands.is_empty() {
        println!("ℹ️  No commands available");
        return;
    }

    println!("📚 Available commands:");
    println!();

    // Group commands by category
    let mut categorized: std::collections::HashMap<
        Option<String>,
        Vec<&crate::llm::CommandMetadata>,
    > = std::collections::HashMap::new();

    for cmd in commands {
        categorized
            .entry(cmd.category.clone())
            .or_default()
            .push(cmd);
    }

    for (category, cmds) in categorized {
        if let Some(cat) = category {
            println!("{}🔖 {}:{}", yellow_color(), cat, reset_color());
        } else {
            println!("{}🔖 General:{}", yellow_color(), reset_color());
        }

        for cmd in cmds {
            println!("   • {} - {}", cmd.id, cmd.summary);
            if let Some(syntax) = &cmd.syntax {
                println!("     Syntax: {}", syntax);
            }
        }
        println!();
    }
}

/// Get color code for confidence score
fn get_confidence_color(confidence: f32) -> String {
    if confidence >= 0.8 {
        green_color()
    } else if confidence >= 0.6 {
        yellow_color()
    } else {
        red_color()
    }
}

/// ANSI color codes
fn green_color() -> String {
    if crate::cli_mode::should_color_output() {
        "\x1b[32m".to_string()
    } else {
        String::new()
    }
}

fn yellow_color() -> String {
    if crate::cli_mode::should_color_output() {
        "\x1b[33m".to_string()
    } else {
        String::new()
    }
}

fn red_color() -> String {
    if crate::cli_mode::should_color_output() {
        "\x1b[31m".to_string()
    } else {
        String::new()
    }
}

fn reset_color() -> String {
    if crate::cli_mode::should_color_output() {
        "\x1b[0m".to_string()
    } else {
        String::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::command::CommandArgs;
    use crate::llm::CommandResolution;

    #[test]
    fn test_confidence_colors() {
        // Test high confidence
        let high_conf = get_confidence_color(0.9);
        assert!(high_conf.contains("[32m") || high_conf.is_empty());

        // Test medium confidence
        let medium_conf = get_confidence_color(0.7);
        assert!(medium_conf.contains("[33m") || medium_conf.is_empty());

        // Test low confidence
        let low_conf = get_confidence_color(0.4);
        assert!(low_conf.contains("[31m") || low_conf.is_empty());
    }

    #[test]
    fn test_resolution_display() {
        let resolution = CommandResolution {
            command_id: "deploy".to_string(),
            args: CommandArgs {
                positional: vec!["app".to_string()],
                named: [("env".to_string(), "prod".to_string())]
                    .into_iter()
                    .collect(),
            },
            confidence: 0.85,
            reasoning: Some("User wants to deploy to production".to_string()),
        };

        // This would normally print to stdout, but we can't easily test that
        display_resolution(&resolution);
    }

    #[test]
    fn test_confirmation_display() {
        let resolution = CommandResolution {
            command_id: "restart".to_string(),
            args: CommandArgs::default(),
            confidence: 0.95,
            reasoning: None,
        };

        display_confirmation(&resolution, Some("Critical service restart"));
    }
}
