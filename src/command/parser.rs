//! Command parser for parsing command syntax (:command arg=value)

use std::collections::HashMap;

/// A parsed chat-style command (`:command arg=value`).
pub struct ParsedCommand {
    pub name: String,
    pub positional: Vec<String>,
    pub named: HashMap<String, String>,
}

/// Parse a command string (e.g., ":restart service=api env=prod").
/// Returns `None` if input doesn't start with `:`.
pub fn parse_command(input: &str) -> Option<ParsedCommand> {
    let input = input.trim();
    if !input.starts_with(':') {
        return None;
    }

    let parts: Vec<&str> = input[1..].split_whitespace().collect();
    if parts.is_empty() {
        return None;
    }

    let name = parts[0].to_string();
    let mut positional = Vec::new();
    let mut named = HashMap::new();

    for part in parts.iter().skip(1) {
        if let Some((key, value)) = part.split_once('=') {
            named.insert(key.to_string(), value.to_string());
        } else {
            positional.push(part.to_string());
        }
    }

    Some(ParsedCommand {
        name,
        positional,
        named,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_simple_command() {
        let result = parse_command(":restart");
        assert!(result.is_some());
        let cmd = result.unwrap();
        assert_eq!(cmd.name, "restart");
        assert!(cmd.positional.is_empty());
        assert!(cmd.named.is_empty());
    }

    #[test]
    fn test_parse_command_with_args() {
        let result = parse_command(":restart service=api env=prod");
        assert!(result.is_some());
        let cmd = result.unwrap();
        assert_eq!(cmd.name, "restart");
        assert_eq!(cmd.named.get("service"), Some(&"api".to_string()));
        assert_eq!(cmd.named.get("env"), Some(&"prod".to_string()));
    }
}
