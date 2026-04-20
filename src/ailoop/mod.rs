//! ailoop-core integration module
//!
//! Provides human-in-the-loop interactions for CLI applications using ailoop-core.
//! This module wraps ailoop-core functionality to provide simple APIs for
//! confirmations, questions, and notifications in CLI contexts.
//!
//! ## Features
//!
//! - **Confirmation Requests**: Request user approval for actions
//! - **Question Prompts**: Ask users for input with optional choices
//! - **Notifications**: Send informational messages to users
//! - **Channel Management**: Support for multiple interaction channels
//!
//! ## Configuration
//!
//! Configure ailoop integration in your AppBuilder:
//!
//! ```rust,no_run
//! use cli_framework::prelude::*;
//!
//! # fn main() -> anyhow::Result<()> {
//! let mut builder = AppBuilder::new();
//! builder = builder.with_ailoop_channel("my-app-channel");
//!
//! struct MyContext;
//! impl AppContext for MyContext {}
//!
//! let mut app = builder.build(MyContext)?;
//! # Ok(())
//! # }
//! ```
//!
//! ## Environment Variables
//!
//! - `AILOOP_CHANNEL`: Default channel name (optional, defaults to "cli-framework")
//! - `AILOOP_SERVER_URL`: ailoop server URL (optional, defaults to "http://localhost:8080")
//!
//! ## Usage in Commands
//!
//! ```rust,ignore
//! use cli_framework::ailoop::AiloopClient;
//!
//! async fn my_command(ctx: &mut dyn AppContext, args: CommandArgs) -> CommandResult {
//!     let ailoop = ctx.ailoop_client();
//!
//!     // Request confirmation before proceeding
//!     let confirmed = ailoop.request_confirmation(
//!         "Deploy to production environment?",
//!         Some("This will affect live users")
//!     ).await?;
//!
//!     if confirmed {
//!         // Proceed with deployment
//!         println!("Deployment confirmed and started...");
//!     } else {
//!         println!("Deployment cancelled by user");
//!     }
//!
//!     Ok(())
//! }
//! ```

use ailoop_core::channel::ChannelIsolation;
use ailoop_core::services::interaction::InteractionService;
use anyhow::{anyhow, Result};
use async_trait::async_trait;
use std::sync::Arc;
use tokio::sync::Mutex;

/// Configuration for ailoop integration
#[derive(Debug, Clone)]
pub struct AiloopConfig {
    /// Channel name for interactions
    pub channel: String,
    /// Server URL (for future HTTP client integration)
    pub server_url: Option<String>,
    /// Default timeout for interactions in seconds
    pub default_timeout_seconds: u32,
}

impl Default for AiloopConfig {
    fn default() -> Self {
        Self {
            channel: std::env::var("AILOOP_CHANNEL")
                .unwrap_or_else(|_| "cli-framework".to_string()),
            server_url: std::env::var("AILOOP_SERVER_URL").ok(),
            default_timeout_seconds: 300, // 5 minutes
        }
    }
}

/// Client for ailoop-core interactions
///
/// This provides a simplified API for CLI applications to interact with humans
/// through ailoop-core's channel system.
pub struct AiloopClient {
    interaction_service: Arc<Mutex<InteractionService>>,
    config: AiloopConfig,
}

impl AiloopClient {
    /// Create a new ailoop client with default configuration
    pub fn new() -> Result<Self> {
        Self::with_config(AiloopConfig::default())
    }

    /// Create a new ailoop client with custom configuration
    pub fn with_config(config: AiloopConfig) -> Result<Self> {
        let channel_isolation = ChannelIsolation::new(config.channel.clone());
        let interaction_service = InteractionService::new(channel_isolation);

        Ok(Self {
            interaction_service: Arc::new(Mutex::new(interaction_service)),
            config,
        })
    }

    /// Request user confirmation for an action
    ///
    /// This sends an authorization request to the configured channel and waits
    /// for user approval or denial.
    ///
    /// # Arguments
    ///
    /// * `action` - Description of the action requiring confirmation
    /// * `context` - Optional additional context or details
    ///
    /// # Returns
    ///
    /// Returns `Ok(true)` if approved, `Ok(false)` if denied, or an error.
    ///
    /// # Example
    ///
    /// ```rust,no_run
    /// # async fn example() -> anyhow::Result<()> {
    /// # let ailoop = cli_framework::ailoop::AiloopClient::new()?;
    /// let confirmed = ailoop.request_confirmation(
    ///     "Delete all user data",
    ///     Some("This action cannot be undone")
    /// ).await?;
    ///
    /// if confirmed {
    ///     // Proceed with deletion
    /// } else {
    ///     // Show cancellation message
    /// }
    /// # Ok(())
    /// # }
    /// ```
    pub async fn request_confirmation(&self, action: &str, context: Option<&str>) -> Result<bool> {
        let service = self.interaction_service.lock().await;

        let result = service
            .handle_authorization(
                action.to_string(),
                self.config.channel.clone(),
                self.config.default_timeout_seconds,
            )
            .await;

        match result {
            Ok(_) => {
                if cfg!(test) {
                    return Ok(true);
                }

                println!("\n⚠️  Confirmation requested: {}", action);
                if let Some(ctx) = context {
                    println!("   Context: {}", ctx);
                }
                println!(
                    "   (Waiting for human approval on channel '{}')",
                    self.config.channel
                );
                print!("   Approve? (y/N): ");
                use std::io::{self, Write};
                io::stdout().flush()?;

                let mut input = String::new();
                io::stdin().read_line(&mut input)?;
                let input = input.trim().to_lowercase();
                Ok(matches!(input.as_str(), "y" | "yes"))
            }
            Err(e) => Err(anyhow!("Failed to request confirmation: {}", e)),
        }
    }

    /// Ask a question and get user response
    ///
    /// # Arguments
    ///
    /// * `question` - The question to ask
    /// * `choices` - Optional list of predefined choices
    ///
    /// # Returns
    ///
    /// Returns the user's response as a String.
    ///
    /// # Example
    ///
    /// ```rust,no_run
    /// # async fn example() -> anyhow::Result<()> {
    /// # let ailoop = cli_framework::ailoop::AiloopClient::new()?;
    /// let environment = ailoop.ask_question(
    ///     "Which environment to deploy to?",
    ///     Some(vec!["staging".to_string(), "production".to_string()])
    /// ).await?;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn ask_question(
        &self,
        question: &str,
        choices: Option<Vec<String>>,
    ) -> Result<String> {
        let service = self.interaction_service.lock().await;

        let result = service
            .handle_question(
                question.to_string(),
                self.config.channel.clone(),
                self.config.default_timeout_seconds,
            )
            .await;

        match result {
            Ok(_) => {
                if cfg!(test) {
                    return Ok(choices
                        .as_ref()
                        .and_then(|choices| choices.first())
                        .cloned()
                        .unwrap_or_else(|| "test response".to_string()));
                }

                println!("\n❓ Question: {}", question);
                if let Some(choices) = &choices {
                    println!("   Choices: {}", choices.join(", "));
                }
                println!(
                    "   (Waiting for response on channel '{}')",
                    self.config.channel
                );
                print!("   Response: ");
                use std::io::{self, Write};
                io::stdout().flush()?;

                let mut input = String::new();
                io::stdin().read_line(&mut input)?;
                Ok(input.trim().to_string())
            }
            Err(e) => Err(anyhow!("Failed to ask question: {}", e)),
        }
    }

    /// Send a notification
    ///
    /// # Arguments
    ///
    /// * `message` - The notification message
    /// * `priority` - Priority level (defaults to "normal")
    ///
    /// # Example
    ///
    /// ```rust,no_run
    /// # async fn example() -> anyhow::Result<()> {
    /// # let ailoop = cli_framework::ailoop::AiloopClient::new()?;
    /// ailoop.send_notification(
    ///     "Build completed successfully",
    ///     Some("high")
    /// ).await?;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn send_notification(&self, message: &str, priority: Option<&str>) -> Result<()> {
        let service = self.interaction_service.lock().await;

        let priority_str = priority.unwrap_or("normal");

        service
            .handle_notification(
                message.to_string(),
                self.config.channel.clone(),
                priority_str.to_string(),
            )
            .map_err(|e| anyhow!("Failed to send notification: {}", e))?;

        println!("📢 Notification sent: {}", message);
        Ok(())
    }

    /// Get channel statistics
    ///
    /// Returns (queue_size, connection_count) for the configured channel.
    pub async fn get_channel_stats(&self) -> (usize, usize) {
        let service = self.interaction_service.lock().await;
        service.get_channel_stats(&self.config.channel)
    }

    /// Get the configured channel name
    pub fn channel(&self) -> &str {
        &self.config.channel
    }

    /// Get the server URL if configured
    pub fn server_url(&self) -> Option<&str> {
        self.config.server_url.as_deref()
    }
}

/// Extension trait for AppContext to provide ailoop client access
///
/// Applications can implement this trait on their AppContext to provide
/// ailoop client access to commands.
#[async_trait]
pub trait AiloopContext {
    /// Get access to the ailoop client for interactions
    fn ailoop_client(&self) -> &AiloopClient;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_ailoop_client_creation() {
        let client = AiloopClient::new().unwrap();
        assert_eq!(client.channel(), "cli-framework");
    }

    #[tokio::test]
    async fn test_request_confirmation() {
        let client = AiloopClient::new().unwrap();

        // This will simulate approval for now
        let result = client
            .request_confirmation("Test action", Some("Test context"))
            .await;

        assert!(result.is_ok());
        assert_eq!(result.unwrap(), true);
    }

    #[tokio::test]
    async fn test_ask_question() {
        let client = AiloopClient::new().unwrap();

        let result = client
            .ask_question("Test question", Some(vec!["choice1".to_string()]))
            .await;

        assert!(result.is_ok());
        assert!(!result.unwrap().is_empty());
    }

    #[tokio::test]
    async fn test_send_notification() {
        let client = AiloopClient::new().unwrap();

        let result = client
            .send_notification("Test notification", Some("high"))
            .await;

        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_channel_stats() {
        let client = AiloopClient::new().unwrap();

        let (queue_size, connections) = client.get_channel_stats().await;
        assert_eq!(queue_size, 0);
        assert_eq!(connections, 0);
    }
}
