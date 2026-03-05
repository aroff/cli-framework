//! Plugin system for extending CLI applications
//!
//! Provides a registry-based plugin system that allows third-party commands
//! to be registered and loaded dynamically.

pub mod manifest;
pub mod registry;

pub use manifest::PluginManifest;
pub use registry::{PluginRegistryConfig, PluginRegistryManager, PluginEntry};