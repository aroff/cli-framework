//! Contract test infrastructure
//!
//! This module provides the test runner and trait contract validators for
//! View, DataSource, Module, and AppBuilder traits.

pub mod view_trait;
pub mod data_source_trait;
pub mod module_trait;
pub mod app_builder_api;

// Async contract tests
mod test_async_data_source;
mod test_async_view;
mod test_async_command;

