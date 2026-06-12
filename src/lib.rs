//! # CLI Framework
//!
//! A pure CLI framework with AI-powered command resolution and plugin system.
//!
//! This framework provides command execution, plugin system, ailoop-core integration
//! for human-in-the-loop interactions, and CLI output utilities (tables, JSON, messages,
//! progress indicators) so application authors can focus on implementing commands
//! rather than CLI infrastructure.
//!
//! ## Features
//!
//! - **Chat Command**: Multi-turn agentic command resolution via aikit-agent (default feature)
//! - **Plugin System**: Registry-based plugin loading with manifest files
//! - **ailoop-core Integration**: Human-in-the-loop confirmations and interactions
//! - **Command Registry**: Centralized command management
//! - **CLI Output**: Rich formatting for tables, JSON, progress, and interactive prompts
//!
//! ## Example
//!
//! ```no_run
//! use cli_framework::prelude::*;
//! use std::collections::HashMap;
//!
//! #[tokio::main]
//! async fn main() -> anyhow::Result<()> {
//!     let builder = AppBuilder::new()
//!         .with_version("myapp", "1.0.0");
//!
//!     // Register typed commands via register::<T>()
//!     // (requires implementing IntoCommandSpec + FromArgValueMap, or #[derive(CommandSpec)])
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

// Shim module to provide `tower::util::BoxCloneLayer` as required by the `api-server` API surface.
#[cfg(feature = "api-server")]
pub mod tower;

// Project config — optional project-root discovery and TOML loading
#[cfg(feature = "project-config")]
pub mod project_config;

// Emulation support — mock executors and test harnesses
#[cfg(feature = "emulation")]
pub mod emulation;

/// Re-export the exit-code marker for parse/usage errors (spec 012 §R5).
pub use app::UsageError;

/// Re-export the `#[derive(CommandSpec)]` macro when the `derive` feature is enabled.
#[cfg(feature = "derive")]
pub use cli_framework_macros::CommandSpec;

/// Construct a `CommandPath` from string literals.
///
/// ```rust
/// use cli_framework::path;
/// let p = path!["skillopt", "run"];
/// assert_eq!(p.to_path_string(), "skillopt/run");
/// ```
///
/// Panics at runtime if any segment contains `'/'` (an invalid segment).
#[macro_export]
macro_rules! path {
    [$($seg:expr),+ $(,)?] => {
        $crate::spec::command_tree::CommandPath::new(&[$($seg),+])
            .expect("path! segments must not contain '/'")
    };
}

/// Prelude module for convenient imports
pub mod prelude {
    pub use crate::app::{AppBuilder, AppContext, AppMeta, UsageError};
    pub use crate::command::{
        Command, FromArgValueMap, IntoCommandSpec, TypedArgs, UiCsp, UiToolMeta,
    };
    pub use crate::message::{AppMessage, AppMessageKind};
    pub use crate::path;
    pub use crate::plugin::PluginRegistryManager;
    pub use crate::spec::{ArgSpec, ArgValue, CommandPath, CommandSpec};

    #[cfg(feature = "chat")]
    pub use crate::command::chat::ChatToolPolicy;

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

#[cfg(feature = "observability")]
pub fn init_default_logging() {
    use tracing_subscriber::EnvFilter;

    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));

    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_target(true)
        .init();
}
