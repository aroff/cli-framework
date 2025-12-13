//! # TUI Framework
//!
//! An opinionated TUI framework library for building terminal user interfaces.
//!
//! This framework provides a complete async event loop, layout system, navigation, status bar,
//! help overlay, command palette, standard widgets (GridView, LogView, ModalView), and
//! CLI output utilities (tables, JSON, messages, progress indicators)
//! so application authors can focus on implementing views, datasources, and commands
//! rather than terminal management.
//!
//! ## Async Runtime
//!
//! The framework uses [Tokio](https://tokio.rs/) as its async runtime and manages the runtime
//! internally. Applications do not need to initialize Tokio themselves - the framework handles
//! all async runtime setup automatically. All trait methods (DataSource, View, Command) support
//! async operations using `.await`, enabling direct integration with async service clients
//! (e.g., reqwest, tokio-postgres) without blocking the UI.
//!
//! ## Example
//!
//! ```no_run
//! use async_trait::async_trait;
//! use tui_framework::prelude::*;
//! use tui_framework::view::{View, ViewResult, HelpItem};
//! use crossterm::event::Event;
//! use ratatui::layout::Rect;
//! use ratatui::Frame;
//!
//! // Define a simple view
//! struct MyView;
//! #[async_trait]
//! impl View for MyView {
//!     fn id(&self) -> &'static str { "my.view" }
//!     fn title(&self) -> &'static str { "My View" }
//!     fn render(&mut self, _f: &mut Frame, _area: Rect, _ctx: &dyn AppContext) {}
//!     async fn handle_event(&mut self, _event: &Event, _ctx: &mut dyn AppContext) -> ViewResult {
//!         ViewResult::Ignored
//!     }
//!     fn help_items(&self) -> Vec<HelpItem> { vec![] }
//! }
//!
//! // Build and run the app
//! struct MyContext;
//! impl AppContext for MyContext {}
//!
//! # #[tokio::main]
//! # async fn main() -> anyhow::Result<()> {
//! let mut builder = AppBuilder::new();
//! builder = builder
//!     .register_view(MyView)
//!     .map_view_slot(ViewSlot::Slot1, "my.view");
//! let mut app = builder.build(MyContext)?;
//! app.run().await?;
//! # Ok(())
//! # }
//! ```

pub mod app;
pub mod cli_mode;
pub mod cli_output;
pub mod command;
pub mod data_source;
pub mod keymap;
pub mod message;
pub mod view;
pub mod widget;

// Optional modules
#[cfg(feature = "observability")]
pub mod observability;

pub mod auth;
pub mod progress_formatting;
pub mod retry;

// HTTP retry integration module
// Note: This module requires applications to provide `reqwest` dependency
// Applications should add reqwest to their Cargo.toml when using this module
pub mod http_retry;

/// Prelude module for convenient imports
pub mod prelude {
    pub use crate::app::{AppBuilder, AppContext};
    pub use crate::command::{Command, CommandArgs};
    pub use crate::data_source::DataSource;
    pub use crate::keymap::{KeyBinding, KeymapConfig, ViewSlot};
    pub use crate::message::{AppMessage, AppMessageKind};
    pub use crate::view::View;
}
