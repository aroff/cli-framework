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
    display_resolution_to(resolution, &mut std::io::stdout());
}

/// Writer-based variant of `display_resolution` for testing.
pub fn display_resolution_to<W: std::io::Write>(resolution: &CommandResolution, w: &mut W) {
    writeln!(w, "\n🤔 Resolving command...").ok();

    let confidence_color = get_confidence_color(resolution.confidence);
    let safe_id = sanitize_untrusted_output(&resolution.command_id);
    writeln!(
        w,
        "🎯 Resolved: {} (confidence: {}{:.1}%{})",
        safe_id,
        confidence_color,
        resolution.confidence * 100.0,
        reset_color()
    )
    .ok();

    if !resolution.args.positional.is_empty() || !resolution.args.named.is_empty() {
        writeln!(w, "📋 Arguments:").ok();
        if !resolution.args.positional.is_empty() {
            let safe_positional: Vec<String> = resolution
                .args
                .positional
                .iter()
                .map(|s| sanitize_untrusted_output(s))
                .collect();
            writeln!(w, "   Positional: {:?}", safe_positional).ok();
        }
        if !resolution.args.named.is_empty() {
            let safe_named: std::collections::HashMap<String, String> = resolution
                .args
                .named
                .iter()
                .map(|(k, v)| (sanitize_untrusted_output(k), sanitize_untrusted_output(v)))
                .collect();
            writeln!(w, "   Named: {:?}", safe_named).ok();
        }
    }

    if let Some(reasoning) = &resolution.reasoning {
        let safe_reasoning = sanitize_untrusted_output(reasoning);
        writeln!(w, "💭 Reasoning: {}", safe_reasoning).ok();
    }

    writeln!(w).ok();
}

/// Display confirmation prompt
///
/// Shows a formatted confirmation prompt with command details.
pub fn display_confirmation(resolution: &CommandResolution, context: Option<&str>) {
    display_confirmation_to(resolution, context, &mut std::io::stdout());
}

/// Writer-based variant of `display_confirmation` for testing.
pub fn display_confirmation_to<W: std::io::Write>(
    resolution: &CommandResolution,
    context: Option<&str>,
    w: &mut W,
) {
    let safe_id = sanitize_untrusted_output(&resolution.command_id);
    writeln!(w, "⚠️  Execute command: {}", safe_id).ok();

    if !resolution.args.positional.is_empty() {
        let safe_positional: Vec<String> = resolution
            .args
            .positional
            .iter()
            .map(|s| sanitize_untrusted_output(s))
            .collect();
        writeln!(w, "   Positional args: {:?}", safe_positional).ok();
    }

    if !resolution.args.named.is_empty() {
        for (key, value) in &resolution.args.named {
            let safe_key = sanitize_untrusted_output(key);
            let safe_value = sanitize_untrusted_output(value);
            writeln!(w, "   {}: {}", safe_key, safe_value).ok();
        }
    }

    if let Some(ctx) = context {
        let safe_ctx = sanitize_untrusted_output(ctx);
        writeln!(w, "   Context: {}", safe_ctx).ok();
    }

    writeln!(w).ok();
}

/// Display retry information
///
/// Shows information about retry attempts with error details.
pub fn display_retry(attempt: usize, max_attempts: usize, error: &str) {
    display_retry_to(attempt, max_attempts, error, &mut std::io::stdout());
}

/// Writer-based variant of `display_retry` for testing.
pub fn display_retry_to<W: std::io::Write>(
    attempt: usize,
    max_attempts: usize,
    error: &str,
    w: &mut W,
) {
    let attempt_color = if attempt >= max_attempts {
        red_color()
    } else {
        yellow_color()
    };

    writeln!(
        w,
        "{}🔄 Retry {} of {}{}",
        attempt_color,
        attempt,
        max_attempts,
        reset_color()
    )
    .ok();

    let safe_error = sanitize_untrusted_output(error);
    writeln!(w, "❌ Previous attempt failed: {}", safe_error).ok();
    writeln!(w).ok();
}

/// Display successful command execution
///
/// Shows confirmation that the command executed successfully.
pub fn display_success(command_id: &str) {
    display_success_to(command_id, &mut std::io::stdout());
}

/// Writer-based variant of `display_success` for testing.
pub fn display_success_to<W: std::io::Write>(command_id: &str, w: &mut W) {
    let safe_id = sanitize_untrusted_output(command_id);
    writeln!(w, "✅ {} executed successfully", safe_id).ok();
}

/// Display command execution failure
///
/// Shows detailed error information for failed commands.
pub fn display_failure(command_id: &str, error: &str) {
    display_failure_to(command_id, error, &mut std::io::stdout());
}

/// Writer-based variant of `display_failure` for testing.
pub fn display_failure_to<W: std::io::Write>(command_id: &str, error: &str, w: &mut W) {
    let safe_id = sanitize_untrusted_output(command_id);
    let safe_error = sanitize_untrusted_output(error);
    writeln!(w, "❌ {} failed: {}", safe_id, safe_error).ok();
}

/// Display max retries exceeded
///
/// Shows when all retry attempts have been exhausted.
pub fn display_max_retries_exceeded(command_id: &str) {
    display_max_retries_exceeded_to(command_id, &mut std::io::stdout());
}

/// Writer-based variant of `display_max_retries_exceeded` for testing.
pub fn display_max_retries_exceeded_to<W: std::io::Write>(command_id: &str, w: &mut W) {
    let safe_id = sanitize_untrusted_output(command_id);
    writeln!(
        w,
        "{}💥 Max retries exceeded for command: {}{}",
        red_color(),
        safe_id,
        reset_color()
    )
    .ok();
}

/// Display suggested alternative command
///
/// Shows AI-suggested alternative when a command fails.
pub fn display_suggestion(resolution: &CommandResolution) {
    display_suggestion_to(resolution, &mut std::io::stdout());
}

/// Writer-based variant of `display_suggestion` for testing.
pub fn display_suggestion_to<W: std::io::Write>(resolution: &CommandResolution, w: &mut W) {
    writeln!(w, "💡 AI suggests trying:").ok();
    display_resolution_to(resolution, w);
}

/// Display command help for available commands
///
/// Shows a formatted list of available commands for user reference.
pub fn display_command_help(commands: &[crate::llm::CommandMetadata]) {
    display_command_help_to(commands, &mut std::io::stdout());
}

/// Writer-based variant of `display_command_help` for testing.
pub fn display_command_help_to<W: std::io::Write>(
    commands: &[crate::llm::CommandMetadata],
    w: &mut W,
) {
    if commands.is_empty() {
        writeln!(w, "ℹ️  No commands available").ok();
        return;
    }

    writeln!(w, "📚 Available commands:").ok();
    writeln!(w).ok();

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
            writeln!(w, "{}🔖 {}:{}", yellow_color(), cat, reset_color()).ok();
        } else {
            writeln!(w, "{}🔖 General:{}", yellow_color(), reset_color()).ok();
        }

        for cmd in cmds {
            let safe_id = sanitize_untrusted_output(&cmd.id);
            let safe_summary = sanitize_untrusted_output(&cmd.summary);
            writeln!(w, "   • {} - {}", safe_id, safe_summary).ok();
            if let Some(syntax) = &cmd.syntax {
                let safe_syntax = sanitize_untrusted_output(syntax);
                writeln!(w, "     Syntax: {}", safe_syntax).ok();
            }
        }
        writeln!(w).ok();
    }
}

fn get_confidence_color(confidence: f32) -> String {
    if confidence >= 0.8 {
        green_color()
    } else if confidence >= 0.6 {
        yellow_color()
    } else {
        red_color()
    }
}

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
        let high_conf = get_confidence_color(0.9);
        assert!(high_conf.contains("[32m") || high_conf.is_empty());

        let medium_conf = get_confidence_color(0.7);
        assert!(medium_conf.contains("[33m") || medium_conf.is_empty());

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
