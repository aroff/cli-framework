//! Plugin manifest format
//!
//! Defines the structure for plugin manifests that describe available commands
//! and their metadata.

use crate::llm::CommandMetadata;
use serde::{Deserialize, Serialize};

/// Plugin manifest describing available commands
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PluginManifest {
    /// Plugin name
    pub name: String,
    /// Plugin version
    pub version: String,
    /// Plugin description
    pub description: Option<String>,
    /// Plugin author
    pub author: Option<String>,
    /// Available commands
    pub commands: Vec<PluginCommand>,
}

/// Command definition within a plugin
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PluginCommand {
    /// Command ID (unique within plugin)
    pub id: String,
    /// Human-readable name
    pub name: String,
    /// Command description
    pub description: String,
    /// Syntax hint (optional)
    pub syntax: Option<String>,
    /// Category for grouping (optional)
    pub category: Option<String>,
    /// Command execution method
    pub execution: CommandExecution,
}

/// How the command should be executed
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum CommandExecution {
    /// Execute via subprocess (external command)
    #[serde(rename = "subprocess")]
    Subprocess {
        /// Command to execute
        command: String,
        /// Arguments to pass
        args: Vec<String>,
        /// Working directory (optional)
        cwd: Option<String>,
    },
    /// Execute via HTTP API
    #[serde(rename = "http")]
    Http {
        /// HTTP method
        method: String,
        /// URL endpoint
        url: String,
        /// Request headers (optional)
        headers: Option<std::collections::HashMap<String, String>>,
    },
    /// Execute via library call (future extension)
    #[serde(rename = "library")]
    Library {
        /// Library path
        library_path: String,
        /// Function name
        function_name: String,
    },
}

impl PluginManifest {
    /// Load manifest from JSON file
    pub fn from_file(path: &std::path::Path) -> anyhow::Result<Self> {
        let content = std::fs::read_to_string(path)?;
        let manifest: PluginManifest = serde_json::from_str(&content)?;
        Ok(manifest)
    }

    /// Save manifest to JSON file
    pub async fn save_to_file(&self, path: &std::path::Path) -> anyhow::Result<()> {
        let content = serde_json::to_string_pretty(self)?;
        tokio::fs::write(path, content).await?;
        Ok(())
    }

    /// Get command metadata for LLM context
    pub fn get_command_metadata(&self, plugin_id: &str) -> Vec<CommandMetadata> {
        self.commands
            .iter()
            .map(|cmd| CommandMetadata {
                id: format!("{}.{}", plugin_id, cmd.id),
                summary: cmd.description.clone(),
                syntax: cmd.syntax.clone(),
                category: cmd.category.clone(),
            })
            .collect()
    }

    /// Find a command by ID
    pub fn find_command(&self, command_id: &str) -> Option<&PluginCommand> {
        self.commands.iter().find(|cmd| cmd.id == command_id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_manifest_serialization() {
        let manifest = PluginManifest {
            name: "test-plugin".to_string(),
            version: "1.0.0".to_string(),
            description: Some("Test plugin".to_string()),
            author: Some("Test Author".to_string()),
            commands: vec![
                PluginCommand {
                    id: "hello".to_string(),
                    name: "Hello World".to_string(),
                    description: "Print hello world".to_string(),
                    syntax: Some("hello".to_string()),
                    category: Some("demo".to_string()),
                    execution: CommandExecution::Subprocess {
                        command: "echo".to_string(),
                        args: vec!["hello world".to_string()],
                        cwd: None,
                    },
                }
            ],
        };

        let json = serde_json::to_string_pretty(&manifest).unwrap();
        let deserialized: PluginManifest = serde_json::from_str(&json).unwrap();

        assert_eq!(manifest.name, deserialized.name);
        assert_eq!(manifest.version, deserialized.version);
        assert_eq!(manifest.commands.len(), deserialized.commands.len());
    }

    #[test]
    fn test_command_metadata_generation() {
        let manifest = PluginManifest {
            name: "test".to_string(),
            version: "1.0.0".to_string(),
            description: None,
            author: None,
            commands: vec![
                PluginCommand {
                    id: "cmd1".to_string(),
                    name: "Command 1".to_string(),
                    description: "Description 1".to_string(),
                    syntax: Some("cmd1 <arg>".to_string()),
                    category: Some("cat1".to_string()),
                    execution: CommandExecution::Subprocess {
                        command: "echo".to_string(),
                        args: vec![],
                        cwd: None,
                    },
                }
            ],
        };

        let metadata = manifest.get_command_metadata("test-plugin");
        assert_eq!(metadata.len(), 1);
        assert_eq!(metadata[0].id, "test-plugin.cmd1");
        assert_eq!(metadata[0].summary, "Description 1");
        assert_eq!(metadata[0].syntax, Some("cmd1 <arg>".to_string()));
        assert_eq!(metadata[0].category, Some("cat1".to_string()));
    }
}