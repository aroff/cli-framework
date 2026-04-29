//! CLI output utilities for ask command
//!
//! Provides specialized formatting for displaying command resolution results,
//! confidence scores, retry attempts, and error messages.

use crate::llm::CommandResolution;
use crate::security::sanitize_untrusted_output;

/// Display command resolution result
///
/// Shows the resolved command, confidence score, arguments, and reasoning.
pub fn display_resolution(resolution: &CommandResolution) {
    println!("\n🤔 Resolving command...");

    // Show confidence with color coding
    let confidence_color = get_confidence_color(resolution.confidence);
    let safe_id = sanitize_untrusted_output(&resolution.command_id);
    println!(
        "🎯 Resolved: {} (confidence: {}{:.1}%{})",
        safe_id,
        confidence_color,
        resolution.confidence * 100.0,
        reset_color()
    );

    // Show arguments if any
    if !resolution.args.positional.is_empty() || !resolution.args.named.is_empty() {
        println!("📋 Arguments:");
        if !resolution.args.positional.is_empty() {
            let safe_positional: Vec<String> = resolution
                .args
                .positional
                .iter()
                .map(|s| sanitize_untrusted_output(s))
                .collect();
            println!("   Positional: {:?}", safe_positional);
        }
        if !resolution.args.named.is_empty() {
            let safe_named: std::collections::HashMap<String, String> = resolution
                .args
                .named
                .iter()
                .map(|(k, v)| (sanitize_untrusted_output(k), sanitize_untrusted_output(v)))
                .collect();
            println!("   Named: {:?}", safe_named);
        }
    }

    // Show reasoning if available
    if let Some(reasoning) = &resolution.reasoning {
        let safe_reasoning = sanitize_untrusted_output(reasoning);
        println!("💭 Reasoning: {}", safe_reasoning);
    }

    println!();
}

/// Display confirmation prompt
///
/// Shows a formatted confirmation prompt with command details.
pub fn display_confirmation(resolution: &CommandResolution, context: Option<&str>) {
    let safe_id = sanitize_untrusted_output(&resolution.command_id);
    println!("⚠️  Execute command: {}", safe_id);

    if !resolution.args.positional.is_empty() {
        let safe_positional: Vec<String> = resolution
            .args
            .positional
            .iter()
            .map(|s| sanitize_untrusted_output(s))
            .collect();
        println!("   Positional args: {:?}", safe_positional);
    }

    if !resolution.args.named.is_empty() {
        for (key, value) in &resolution.args.named {
            let safe_key = sanitize_untrusted_output(key);
            let safe_value = sanitize_untrusted_output(value);
            println!("   {}: {}", safe_key, safe_value);
        }
    }

    if let Some(ctx) = context {
        let safe_ctx = sanitize_untrusted_output(ctx);
        println!("   Context: {}", safe_ctx);
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

    let safe_error = sanitize_untrusted_output(error);
    println!("❌ Previous attempt failed: {}", safe_error);
    println!();
}

/// Display successful command execution
///
/// Shows confirmation that the command executed successfully.
pub fn display_success(command_id: &str) {
    let safe_id = sanitize_untrusted_output(command_id);
    println!("✅ {} executed successfully", safe_id);
}

/// Display command execution failure
///
/// Shows detailed error information for failed commands.
pub fn display_failure(command_id: &str, error: &str) {
    let safe_id = sanitize_untrusted_output(command_id);
    let safe_error = sanitize_untrusted_output(error);
    println!("❌ {} failed: {}", safe_id, safe_error);
}

/// Display max retries exceeded
///
/// Shows when all retry attempts have been exhausted.
pub fn display_max_retries_exceeded(command_id: &str) {
    let safe_id = sanitize_untrusted_output(command_id);
    println!(
        "{}💥 Max retries exceeded for command: {}{}",
        red_color(),
        safe_id,
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
            let safe_id = sanitize_untrusted_output(&cmd.id);
            let safe_summary = sanitize_untrusted_output(&cmd.summary);
            println!("   • {} - {}", safe_id, safe_summary);
            if let Some(syntax) = &cmd.syntax {
                let safe_syntax = sanitize_untrusted_output(syntax);
                println!("     Syntax: {}", safe_syntax);
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
