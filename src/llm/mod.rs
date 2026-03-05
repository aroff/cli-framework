//! LLM Provider System
//!
//! Provides a unified interface for different LLM providers to resolve natural language
//! commands into structured command executions.
//!
//! ## Supported Providers
//!
//! - OpenAI (GPT models)
//! - Anthropic (Claude models)
//! - Extensible trait for additional providers
//!
//! ## Environment Variables
//!
//! - `LLM_PROVIDER`: Provider selection ("openai", "anthropic")
//! - `OPENAI_API_KEY`: Required for OpenAI provider
//! - `ANTHROPIC_API_KEY`: Required for Anthropic provider
//! - `LLM_MODEL`: Model selection (defaults to provider-specific defaults)
//!
//! ## Usage
//!
//! ```rust,no_run
//! use cli_framework::llm::{LlmProviderFactory, CommandMetadata};
//!
//! // Create provider from environment
//! let provider = LlmProviderFactory::from_env()?;
//!
//! // Define available commands
//! let commands = vec![
//!     CommandMetadata {
//!         id: "deploy".to_string(),
//!         summary: "Deploy application".to_string(),
//!         syntax: Some("deploy --env <env>".to_string()),
//!         category: Some("deployment".to_string()),
//!     }
//! ];
//!
//! // Resolve natural language query
//! let resolution = provider.resolve_command(
//!     "deploy the app to production",
//!     &commands
//! ).await?;
//!
//! println!("Resolved to command: {}", resolution.command_id);
//! ```

use crate::command::CommandArgs;
use anyhow::Result;
use async_trait::async_trait;

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
    ///
    /// # Arguments
    ///
    /// * `query` - Natural language query (e.g., "deploy to production")
    /// * `available_commands` - List of available commands with metadata
    ///
    /// # Returns
    ///
    /// Returns a CommandResolution with the resolved command and arguments,
    /// or an error if resolution fails.
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
    ///
    /// Reads `LLM_PROVIDER` to determine which provider to use:
    /// - "openai" - OpenAI provider (requires OPENAI_API_KEY)
    /// - "anthropic" - Anthropic provider (requires ANTHROPIC_API_KEY)
    ///
    /// Falls back to OpenAI if LLM_PROVIDER is not set.
    pub fn from_env() -> Result<Box<dyn LlmProvider>> {
        let provider = std::env::var("LLM_PROVIDER")
            .unwrap_or_else(|_| "openai".to_string())
            .to_lowercase();

        match provider.as_str() {
            "openai" => {
                let api_key = std::env::var("OPENAI_API_KEY")
                    .map_err(|_| anyhow::anyhow!("OPENAI_API_KEY environment variable is required for OpenAI provider"))?;
                let model = std::env::var("LLM_MODEL").unwrap_or_else(|_| "gpt-4".to_string());
                Ok(Box::new(OpenAiProvider::new(api_key, model)))
            }
            "anthropic" => {
                let api_key = std::env::var("ANTHROPIC_API_KEY")
                    .map_err(|_| anyhow::anyhow!("ANTHROPIC_API_KEY environment variable is required for Anthropic provider"))?;
                let model = std::env::var("LLM_MODEL").unwrap_or_else(|_| "claude-3-sonnet-20240229".to_string());
                Ok(Box::new(AnthropicProvider::new(api_key, model)))
            }
            _ => Err(anyhow::anyhow!("Unsupported LLM provider: {}. Supported: openai, anthropic", provider)),
        }
    }
}

// Provider implementations
pub mod openai;
pub mod anthropic;

pub use openai::OpenAiProvider;
pub use anthropic::AnthropicProvider;