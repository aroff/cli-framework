//! LLM Provider System
//!
//! Provides a unified interface for different LLM providers to resolve natural language
//! commands into structured command executions.

use crate::command::CommandArgs;
use anyhow::Result;
use async_trait::async_trait;
use std::sync::Arc;

/// Metadata about a command for LLM context
#[derive(Debug, Clone)]
pub struct CommandMetadata {
    /// Unique command identifier
    pub id: String,
    /// Short description of what the command does
    pub summary: String,
    /// Optional syntax hint (e.g., "command --flag <arg>")
    pub syntax: Option<String>,
    /// Optional category for grouping commands
    pub category: Option<String>,
}

/// Resolution result from LLM provider
#[derive(Debug, Clone)]
pub struct CommandResolution {
    /// The resolved command ID
    pub command_id: String,
    /// Parsed command arguments
    pub args: CommandArgs,
    /// Confidence score (0.0 to 1.0)
    pub confidence: f32,
    /// Optional reasoning from the LLM
    pub reasoning: Option<String>,
}

/// Trait for LLM providers that can resolve natural language to commands
#[async_trait]
pub trait LlmProvider: Send + Sync {
    /// Resolve a natural language query into a structured command
    async fn resolve_command(
        &self,
        query: &str,
        available_commands: &[CommandMetadata],
    ) -> Result<CommandResolution>;
}

/// Factory for creating LLM providers from environment configuration
pub struct LlmProviderFactory;

impl LlmProviderFactory {
    /// Create an LLM provider based on environment variables
    pub fn from_env() -> Result<Arc<dyn LlmProvider>> {
        let provider = std::env::var("LLM_PROVIDER")
            .unwrap_or_else(|_| "openai".to_string())
            .to_lowercase();

        match provider.as_str() {
            "openai" => {
                let api_key = std::env::var("OPENAI_API_KEY").map_err(|_| {
                    anyhow::anyhow!(
                        "OPENAI_API_KEY environment variable is required for OpenAI provider"
                    )
                })?;
                let model = std::env::var("LLM_MODEL").unwrap_or_else(|_| "gpt-4".to_string());
                Ok(Arc::new(OpenAiProvider::new(api_key, model)))
            }
            "anthropic" => {
                let api_key = std::env::var("ANTHROPIC_API_KEY").map_err(|_| {
                    anyhow::anyhow!(
                        "ANTHROPIC_API_KEY environment variable is required for Anthropic provider"
                    )
                })?;
                let model = std::env::var("LLM_MODEL")
                    .unwrap_or_else(|_| "claude-3-sonnet-20240229".to_string());
                Ok(Arc::new(AnthropicProvider::new(api_key, model)))
            }
            _ => Err(anyhow::anyhow!(
                "Unsupported LLM provider: {}. Supported: openai, anthropic",
                provider
            )),
        }
    }
}

pub mod anthropic;
pub mod openai;

pub use anthropic::AnthropicProvider;
pub use openai::OpenAiProvider;
