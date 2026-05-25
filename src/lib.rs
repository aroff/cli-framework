//! # CLI Framework
//!
//! A pure CLI framework with AI-powered command resolution and plugin system.
//!
//! This framework provides command execution, LLM-powered natural language command resolution,
//! plugin system, ailoop-core integration for human-in-the-loop interactions, and
//! CLI output utilities (tables, JSON, messages, progress indicators)
//! so application authors can focus on implementing commands
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
//!             expose_mcp: false,
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
pub mod message;

// New modules for CLI framework
pub mod ailoop;
pub mod llm;
pub mod plugin;

// Optional modules
#[cfg(feature = "observability")]
pub mod observability;

pub mod retry;
pub mod security;

// HTTP retry integration module
// Note: This module requires applications to provide `reqwest` dependency
// Applications should add reqwest to their Cargo.toml when using this module
pub mod http_retry;

// Spec and parser modules
pub mod parser;
pub mod spec;

// Command surface export — always compiled, no feature flag
pub mod command_surface;

// Testkit — compile only when the `testkit` feature is active
#[cfg(feature = "testkit")]
pub mod testkit;

// MCP schema + optional server support.
// `mcp-server` gates the server transport and `rmcp`/`axum` deps, but the schema and
// tool descriptor helpers are always available for in-process tool execution (e.g. `chat`).
pub mod mcp;

// Doctor diagnostics framework — compile only when the `doctor` feature is active
#[cfg(feature = "doctor")]
pub mod doctor;

// API server — compile only when the `api-server` feature is active
#[cfg(feature = "api-server")]
pub mod api;

// Re-export axum so consumers use the exact version linked by this crate.
#[cfg(feature = "api-server")]
pub use axum;

// Project config — optional project-root discovery and TOML loading
#[cfg(feature = "project-config")]
pub mod project_config;

/// Prelude module for convenient imports
pub mod prelude {
    pub use crate::app::{AppBuilder, AppContext, AppMeta};
    pub use crate::command::{Command, CommandArgs};
    pub use crate::llm::{CommandMetadata, CommandResolution, LlmProvider};
    pub use crate::message::{AppMessage, AppMessageKind};
    pub use crate::plugin::PluginRegistryManager;
    pub use crate::spec::{ArgSpec, ArgValue, CommandPath, CommandSpec};

    #[cfg(feature = "doctor")]
    pub use crate::doctor::{
        CheckSeverity, DoctorCheck, DoctorFinding, DoctorModule, DoctorReport,
    };

    #[cfg(feature = "project-config")]
    pub use crate::project_config::{
        find_and_load, find_and_load_with_options, find_file_upward, find_file_upward_with_options,
        load_toml_file, load_toml_str, DiscoverOptions, ProjectConfigError, ProjectRoot,
    };
}
