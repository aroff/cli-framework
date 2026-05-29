//! ailoop-core integration module
//!
//! Provides human-in-the-loop interactions for CLI applications using ailoop-core
//! WebSocket client APIs.
//!
//! ## Pairing Requirement
//!
//! This framework depends on a running `ailoop serve` process at the configured
//! `AILOOP_SERVER` URL (default: `ws://localhost:8080`). All HITL operations
//! (`request_confirmation`, `ask_question`, `send_notification`) establish a
//! per-call WebSocket connection to that server. If the server is unreachable,
//! all methods return `Err`; no silent fallback to stdin is permitted.
//!
//! Start a paired server before using HITL features:
//! ```bash
//! ailoop serve --port 8080
//! # or
//! export AILOOP_SERVER=ws://localhost:8080
//! ```
//!
//! ## Configuration
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
//! - `AILOOP_SERVER`: WebSocket URL of the paired ailoop server (defaults to `ws://localhost:8080`)
//! - `AILOOP_CHANNEL`: Default channel name (optional, defaults to "cli-framework")
//!
//! ## Error Semantics
//!
//! All HITL methods return `Err` if the ailoop server is unreachable, times out,
//! or returns an unexpected response. No silent fallback to stdin is permitted.

use anyhow::{anyhow, Result};

/// Configuration for ailoop integration
#[derive(Debug, Clone)]
pub struct AiloopConfig {
    /// Channel name for interactions
    pub channel: String,
    /// Server URL (WebSocket URL; http:// and https:// are normalized to ws:// and wss://)
    pub server_url: Option<String>,
    /// Default timeout for interactions in seconds
    pub default_timeout_seconds: u32,
}

impl Default for AiloopConfig {
    fn default() -> Self {
        Self {
            channel: std::env::var("AILOOP_CHANNEL")
                .unwrap_or_else(|_| "cli-framework".to_string()),
            server_url: std::env::var("AILOOP_SERVER").ok(),
            default_timeout_seconds: 300, // 5 minutes
        }
    }
}

/// Normalize http(s):// URLs to ws(s):// for WebSocket use.
pub fn normalize_ws_url(url: &str) -> String {
    if url.starts_with("http://") {
        url.replacen("http://", "ws://", 1)
    } else if url.starts_with("https://") {
        url.replacen("https://", "wss://", 1)
    } else {
        url.to_string()
    }
}

/// Client for ailoop-core interactions via WebSocket.
///
/// All HITL methods delegate to a paired `ailoop serve` process via
/// `ailoop_core::client` WebSocket APIs. There is no in-process channel state;
/// each method call establishes a new WebSocket connection.
///
/// # Pairing Requirement
///
/// A running `ailoop serve` process at `AILOOP_SERVER` (default: `ws://localhost:8080`)
/// is required for all HITL operations. If the server is unreachable or returns an
/// error, all methods return `Err`.
#[derive(Clone, Debug)]
pub struct AiloopClient {
    config: AiloopConfig,
}

impl AiloopClient {
    /// Create a new ailoop client with default configuration.
    pub fn new() -> Result<Self> {
        Self::with_config(AiloopConfig::default())
    }

    /// Create a new ailoop client with custom configuration.
    ///
    /// Validates that `server_url`, if provided, normalizes to a `ws://` or `wss://` URL.
    /// Returns `Err` with `AILOOP_INVALID_URL` context if the URL cannot be normalized.
    pub fn with_config(config: AiloopConfig) -> Result<Self> {
        if let Some(ref url) = config.server_url {
            let normalized = normalize_ws_url(url);
            if !normalized.starts_with("ws://") && !normalized.starts_with("wss://") {
                return Err(anyhow!("Invalid WebSocket URL: {}", url));
            }
        }
        Ok(Self { config })
    }

    /// Compute the effective server URL: config → AILOOP_SERVER env → default.
    fn effective_server_url(&self) -> String {
        let raw = self.config.server_url.clone().unwrap_or_else(|| {
            std::env::var("AILOOP_SERVER").unwrap_or_else(|_| "ws://localhost:8080".to_string())
        });
        normalize_ws_url(&raw)
    }

    fn timeout_seconds(&self) -> u32 {
        self.config.default_timeout_seconds
    }

    /// Request user confirmation for an action via ailoop WebSocket `authorize`.
    ///
    /// Returns `Ok(true)` if approved, `Ok(false)` if denied, or `Err` on failure.
    /// No fallback to stdin. Requires a paired `ailoop serve` process.
    ///
    /// # Arguments
    ///
    /// * `action` - Description of the action requiring confirmation
    /// * `context` - Optional additional context appended to the action string
    pub async fn request_confirmation(&self, action: &str, context: Option<&str>) -> Result<bool> {
        use ailoop_core::models::{MessageContent, ResponseType};

        let server = self.effective_server_url();
        let action_str = if let Some(ctx) = context {
            format!("{} ({})", action, ctx)
        } else {
            action.to_string()
        };

        let response = ailoop_core::client::authorize(
            &server,
            &self.config.channel,
            &action_str,
            self.timeout_seconds(),
        )
        .await
        .map_err(|e| anyhow!("Ailoop authorization failed: {}", e))?;

        match response {
            None => Err(anyhow!("Ailoop authorization failed: request timed out")),
            Some(msg) => match msg.content {
                MessageContent::Response { response_type, .. } => match response_type {
                    ResponseType::AuthorizationApproved => Ok(true),
                    ResponseType::AuthorizationDenied => Ok(false),
                    ResponseType::Timeout => Err(anyhow!("Ailoop authorization failed: timed out")),
                    ResponseType::Cancelled => {
                        Err(anyhow!("Ailoop authorization failed: cancelled"))
                    }
                    other => Err(anyhow!(
                        "Ailoop authorization failed: unexpected response type {:?}",
                        other
                    )),
                },
                _ => Err(anyhow!(
                    "Ailoop authorization failed: unexpected message content"
                )),
            },
        }
    }

    /// Ask a structured multi-option question via ailoop WebSocket `ask_decision`
    /// (`MessageContent::Decision`).
    ///
    /// Returns the selected option as a `String` (canonical option id from the
    /// server, matching the provided choice when ids are the choice text).
    /// No fallback to stdin. Requires a paired `ailoop serve` process.
    ///
    /// **Requires** `choices` with **at least two** non-empty options. Open-ended
    /// prompts without fixed choices are not supported; use `request_confirmation`
    /// or another flow.
    pub async fn ask_question(
        &self,
        question: &str,
        choices: Option<Vec<String>>,
    ) -> Result<String> {
        use ailoop_core::models::{DecisionOption, MessageContent, ResponseType};

        let Some(choice_list) = choices else {
            return Err(anyhow!(
                "Ailoop question failed: at least two choices are required; \
                 open-ended questions are not supported with the current ailoop protocol"
            ));
        };
        let trimmed: Vec<String> = choice_list
            .into_iter()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect();
        if trimmed.len() < 2 {
            return Err(anyhow!(
                "Ailoop question failed: at least two non-empty choices are required; got {}",
                trimmed.len()
            ));
        }
        let mut seen = std::collections::HashSet::new();
        for c in &trimmed {
            if !seen.insert(c.as_str()) {
                return Err(anyhow!("Ailoop question failed: duplicate choice {:?}", c));
            }
        }

        let options: Vec<DecisionOption> = trimmed
            .iter()
            .map(|c| DecisionOption {
                id: c.clone(),
                label: c.clone(),
                detail_markdown: None,
            })
            .collect();

        const MAX_SUMMARY: usize = 500;
        let q = question.trim();
        let (summary, context_markdown) = if q.len() <= MAX_SUMMARY {
            (q.to_string(), None)
        } else {
            let summary: String = q.chars().take(MAX_SUMMARY).collect();
            (summary, Some(q.to_string()))
        };

        let decision_id = format!("cli-framework-{}", uuid::Uuid::new_v4());
        let server = self.effective_server_url();
        let timeout = self.timeout_seconds();

        let response = ailoop_core::client::ask_decision(
            &server,
            &self.config.channel,
            decision_id,
            summary,
            context_markdown,
            options,
            None,
            timeout,
        )
        .await
        .map_err(|e| anyhow!("Ailoop question failed: {}", e))?;

        match response {
            None => Err(anyhow!("Ailoop question failed: request timed out")),
            Some(msg) => match msg.content {
                MessageContent::Response {
                    answer,
                    response_type,
                } => match response_type {
                    ResponseType::Text => {
                        let a = answer
                            .ok_or_else(|| anyhow!("Ailoop question failed: empty answer"))?;
                        Ok(a)
                    }
                    ResponseType::Timeout => Err(anyhow!("Ailoop question failed: timed out")),
                    ResponseType::Cancelled => Err(anyhow!("Ailoop question failed: cancelled")),
                    other => Err(anyhow!(
                        "Ailoop question failed: unexpected response type {:?}",
                        other
                    )),
                },
                _ => Err(anyhow!(
                    "Ailoop question failed: unexpected message content"
                )),
            },
        }
    }

    /// Send a notification via ailoop WebSocket `say`.
    ///
    /// Fire-and-forget: does not wait for a response. Returns `Err` if the
    /// server is unreachable. Requires a paired `ailoop serve` process.
    ///
    /// Note: `priority` is forwarded to the server. If `None`, defaults to "normal".
    pub async fn send_notification(&self, message: &str, priority: Option<&str>) -> Result<()> {
        let server = self.effective_server_url();
        let priority_str = priority.unwrap_or("normal");

        ailoop_core::client::say(&server, &self.config.channel, message, priority_str)
            .await
            .map_err(|e| anyhow!("Ailoop notification failed: {}", e))
    }

    /// Get the configured channel name.
    pub fn channel(&self) -> &str {
        &self.config.channel
    }

    /// Get the raw server URL if explicitly configured (not normalized).
    pub fn server_url(&self) -> Option<&str> {
        self.config.server_url.as_deref()
    }
}

/// Extension trait for AppContext to provide ailoop client access.
///
/// Applications can implement this trait on their AppContext to provide
/// ailoop client access to commands.
pub trait AiloopContext {
    /// Get access to the ailoop client for interactions.
    fn ailoop_client(&self) -> Option<&AiloopClient>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_normalize_ws_url_http() {
        assert_eq!(
            normalize_ws_url("http://localhost:8080"),
            "ws://localhost:8080"
        );
    }

    #[test]
    fn test_normalize_ws_url_https() {
        assert_eq!(normalize_ws_url("https://example.com"), "wss://example.com");
    }

    #[test]
    fn test_normalize_ws_url_ws_passthrough() {
        assert_eq!(
            normalize_ws_url("ws://localhost:8080"),
            "ws://localhost:8080"
        );
    }

    #[test]
    fn test_normalize_ws_url_wss_passthrough() {
        assert_eq!(normalize_ws_url("wss://example.com"), "wss://example.com");
    }

    #[test]
    fn test_ailoop_client_creation() {
        let client = AiloopClient::new().unwrap();
        assert_eq!(client.channel(), "cli-framework");
    }

    #[test]
    fn test_ailoop_client_with_valid_ws_url() {
        let config = AiloopConfig {
            channel: "test".to_string(),
            server_url: Some("ws://localhost:9000".to_string()),
            default_timeout_seconds: 30,
        };
        assert!(AiloopClient::with_config(config).is_ok());
    }

    #[test]
    fn test_ailoop_client_with_http_url_normalized() {
        let config = AiloopConfig {
            channel: "test".to_string(),
            server_url: Some("http://localhost:9000".to_string()),
            default_timeout_seconds: 30,
        };
        assert!(AiloopClient::with_config(config).is_ok());
    }

    #[test]
    fn test_ailoop_client_with_invalid_url() {
        let config = AiloopConfig {
            channel: "test".to_string(),
            server_url: Some("ftp://localhost:9000".to_string()),
            default_timeout_seconds: 30,
        };
        let result = AiloopClient::with_config(config);
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("Invalid WebSocket URL"));
    }

    #[test]
    fn test_effective_server_url_from_config() {
        let config = AiloopConfig {
            channel: "test".to_string(),
            server_url: Some("http://192.168.1.100:9000".to_string()),
            default_timeout_seconds: 30,
        };
        let client = AiloopClient::with_config(config).unwrap();
        assert_eq!(client.effective_server_url(), "ws://192.168.1.100:9000");
    }

    #[test]
    fn test_effective_server_url_default() {
        // Without AILOOP_SERVER set, should default to ws://localhost:8080
        let saved = std::env::var("AILOOP_SERVER").ok();
        std::env::remove_var("AILOOP_SERVER");

        let config = AiloopConfig {
            channel: "test".to_string(),
            server_url: None,
            default_timeout_seconds: 30,
        };
        let client = AiloopClient::with_config(config).unwrap();
        assert_eq!(client.effective_server_url(), "ws://localhost:8080");

        if let Some(v) = saved {
            std::env::set_var("AILOOP_SERVER", v);
        }
    }
}
