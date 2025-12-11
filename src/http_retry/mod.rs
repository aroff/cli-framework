//! HTTP retry integration module
//!
//! Provides automatic retry logic for HTTP requests using the framework's retry infrastructure.
//!
//! This module wraps HTTP clients (specifically `reqwest::Client`) with configurable retry policies,
//! smart error classification, and support for Retry-After headers.
//!
//! # Example
//!
//! ```rust,no_run
//! use reqwest::Client;
//! use tui_framework::http_retry::RetryableHttpClient;
//!
//! # async fn example() -> anyhow::Result<()> {
//! let client = Client::new();
//! let retry_client = RetryableHttpClient::new(client);
//!
//! let response = retry_client.get("https://api.example.com/data").await?;
//! # Ok(())
//! # }
//! ```

pub mod http_errors;

// Re-export for convenience
pub use http_errors::{is_connection_error, is_retryable_http_error, is_timeout};

pub mod client;
pub use client::RetryableHttpClient;
