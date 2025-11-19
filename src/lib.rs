//! # TUI Framework
//!
//! An opinionated TUI framework library for building terminal user interfaces.
//!
//! This framework provides a complete event loop, layout system, navigation, status bar,
//! help overlay, command palette, and standard widgets (GridView, LogView, ModalView)
//! so application authors can focus on implementing views, datasources, and commands
//! rather than terminal management.
//!
//! ## Example
//!
//! ```no_run
//! use tui_framework::prelude::*;
//! use tui_framework::view::{View, ViewResult, HelpItem};
//! use crossterm::event::Event;
//! use ratatui::layout::Rect;
//! use ratatui::Frame;
//!
//! // Define a simple view
//! struct MyView;
//! impl View for MyView {
//!     fn id(&self) -> &'static str { "my.view" }
//!     fn title(&self) -> &'static str { "My View" }
//!     fn render(&mut self, _f: &mut Frame, _area: Rect, _ctx: &dyn AppContext) {}
//!     fn handle_event(&mut self, _event: &Event, _ctx: &mut dyn AppContext) -> ViewResult {
//!         ViewResult::Ignored
//!     }
//!     fn help_items(&self) -> Vec<HelpItem> { vec![] }
//! }
//!
//! // Build and run the app
//! struct MyContext;
//! impl AppContext for MyContext {}
//!
//! # fn main() -> anyhow::Result<()> {
//! let mut builder = AppBuilder::new();
//! builder = builder
//!     .register_view(MyView)
//!     .map_view_slot(ViewSlot::F1, "my.view");
//! let mut app = builder.build(MyContext)?;
//! app.run()?;
//! # Ok(())
//! # }
//! ```

pub mod app;
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
pub mod retry;

/// Prelude module for convenient imports
pub mod prelude {
    pub use crate::app::{AppBuilder, AppContext};
    pub use crate::command::{Command, CommandArgs};
    pub use crate::data_source::DataSource;
    pub use crate::keymap::{KeyBinding, KeymapConfig, ViewSlot};
    pub use crate::message::{AppMessage, AppMessageKind};
    pub use crate::view::View;
}

