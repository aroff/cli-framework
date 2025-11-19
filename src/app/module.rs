//! Module trait for internal modularization
//!
//! Allows applications to group related views, commands, and keybindings
//! into modules for better organization.

use crate::app::builder::AppBuilder;
use anyhow::Result;

/// Trait for application modules
///
/// Modules allow applications to group related components (views, commands, keybindings)
/// together for better organization. This is useful for large applications that want
/// to organize features into logical groups (e.g., AirflowModule, HetznerModule).
pub trait Module {
    /// Returns a stable identifier for the module
    ///
    /// This should be a compile-time string literal that uniquely identifies
    /// the module within the application.
    fn id(&self) -> &'static str;

    /// Called during application build time to register module components
    ///
    /// This method should register all views, commands, and keybindings
    /// that belong to this module with the provided AppBuilder.
    ///
    /// # Errors
    ///
    /// Returns an error if registration fails (e.g., duplicate view IDs,
    /// conflicting keybindings, etc.)
    fn register(&self, builder: &mut AppBuilder) -> Result<()>;
}

