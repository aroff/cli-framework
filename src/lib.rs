//! # CLI Framework
//!
//! A pure CLI framework with AI-powered command resolution and plugin system.
//!
//! This framework provides command execution, LLM-powered natural language command resolution,
//! plugin system, ailoop-core integration for human-in-the-loop interactions, and
//! CLI output utilities (tables, JSON, messages, progress indicators)
//! so application authors can focus on implementing commands and data sources
//! rather than CLI infrastructure.
//!
//! ## Features
//!
//! - **AI Ask Command**: Natural language command resolution using LLM providers (OpenAI, Anthropic)
//! - **Plugin System**: Registry-based plugin loading with manifest files
//! - **ailoop-core Integration**: Human-in-the-loop confirmations and interactions
//! - **Command Registry**: Centralized command management with metadata collection
//! - **CLI Output**: Rich formatting for tables, JSON, progress, and interactive prompts
//!
//! ## Example
//!
//! ```no_run
//! use cli_framework::prelude::*;
//!
//! #[tokio::main]
//! async fn main() -> anyhow::Result<()> {
//!     let mut builder = AppBuilder::new();
//!     builder = builder
//!         .register_command(Command {
//!             id: "hello",
//!             summary: "Say hello",
//!             syntax: Some("hello --name <name>"),
//!             category: Some("greetings"),
//!             spec: None,
//!             validator: None,
//!             execute: std::sync::Arc::new(|_ctx, args| Box::pin(async move {
//!                 let name = args
//!                     .named
//!                     .get("name")
//!                     .map(String::as_str)
//!                     .unwrap_or("World");
//!                 println!("Hello, {}!", name);
//!                 Ok(())
//!             })),
//!         }).unwrap();
//!
//!     let mut app = builder.build(MyContext)?;
//!     app.run().await?;
//!     Ok(())
//! }
//!
//! struct MyContext;
//! impl AppContext for MyContext {}
//! ```

pub mod app;
pub mod cli_mode;
pub mod cli_output;
pub mod command;
pub mod data_source;
pub mod message;

// New modules for CLI framework
pub mod ailoop;
pub mod llm;
pub mod plugin;

// Optional modules
#[cfg(feature = "observability")]
pub mod observability;

pub mod auth;
pub mod retry;
pub mod security;

// HTTP retry integration module
// Note: This module requires applications to provide `reqwest` dependency
// Applications should add reqwest to their Cargo.toml when using this module
pub mod http_retry;

// Spec and parser modules
pub mod parser;
pub mod spec;

// Testkit — compile only when the `testkit` feature is active
#[cfg(feature = "testkit")]
pub mod testkit;

/// Prelude module for convenient imports
pub mod prelude {
    pub use crate::app::{AppBuilder, AppContext, AppMeta};
    pub use crate::command::{Command, CommandArgs};
    pub use crate::data_source::DataSource;
    pub use crate::llm::{CommandMetadata, CommandResolution, LlmProvider};
    pub use crate::message::{AppMessage, AppMessageKind};
    pub use crate::plugin::PluginRegistryManager;
    pub use crate::spec::{ArgSpec, ArgValue, CommandPath, CommandSpec};
}
