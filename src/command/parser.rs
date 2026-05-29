//! Command parser for parsing command syntax (:command arg=value)

use crate::command::CommandArgs;
use std::collections::HashMap;

/// Parse a command string (e.g., ":restart service=api env=prod")
pub fn parse_command(input: &str) -> Option<(String, CommandArgs)> {
    let input = input.trim();
    if !input.starts_with(':') {
        return None;
    }

    let parts: Vec<&str> = input[1..].split_whitespace().collect();
    if parts.is_empty() {
        return None;
    }

    let command_name = parts[0].to_string();
    let mut positional = Vec::new();
    let mut named = HashMap::new();

    for part in parts.iter().skip(1) {
        if let Some((key, value)) = part.split_once('=') {
            named.insert(key.to_string(), value.to_string());
        } else {
            positional.push(part.to_string());
        }
    }

    Some((
        command_name,
        CommandArgs {
            positional,
            named,
            ..Default::default()
        },
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_simple_command() {
        let result = parse_command(":restart");
        assert!(result.is_some());
        let (name, args) = result.unwrap();
        assert_eq!(name, "restart");
        assert!(args.positional.is_empty());
        assert!(args.named.is_empty());
    }

    #[test]
    fn test_parse_command_with_args() {
        let result = parse_command(":restart service=api env=prod");
        assert!(result.is_some());
        let (name, args) = result.unwrap();
        assert_eq!(name, "restart");
        assert_eq!(args.named.get("service"), Some(&"api".to_string()));
        assert_eq!(args.named.get("env"), Some(&"prod".to_string()));
    }
}
