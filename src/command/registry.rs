//! CommandRegistry for managing commands

use crate::command::Command;
use crate::llm::CommandMetadata;
use std::collections::HashMap;

/// Registry for managing commands
#[derive(Clone)]
pub struct CommandRegistry {
    commands: HashMap<String, Command>,
}

impl CommandRegistry {
    /// Create a new command registry
    pub fn new() -> Self {
        Self {
            commands: HashMap::new(),
        }
    }

    /// Register a command
    pub fn register(&mut self, command: Command) {
        self.commands.insert(command.id.to_string(), command);
    }

    /// Get a command by ID
    pub fn get(&self, id: &str) -> Option<&Command> {
        self.commands.get(id)
    }

    /// Get all commands
    pub fn commands(&self) -> impl Iterator<Item = &Command> {
        self.commands.values()
    }

    /// Collect metadata for all commands for LLM context
    pub fn collect_metadata(&self) -> Vec<CommandMetadata> {
        self.commands
            .values()
            .map(|cmd| CommandMetadata {
                id: cmd.id.to_string(),
                summary: cmd.summary.to_string(),
                syntax: cmd.syntax.map(|s| s.to_string()),
                category: cmd.category.map(|c| c.to_string()),
            })
            .collect()
    }
}

impl Default for CommandRegistry {
    fn default() -> Self {
        Self::new()
    }
}
