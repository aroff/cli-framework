//! DataSource trait definition
//!
//! Defines the DataSource trait for providing tabular data to GridView.

use crate::app::context::AppContext;
use anyhow::Result;
use async_trait::async_trait;

/// DataSource trait for providing tabular data
///
/// # Async Operations
///
/// The `refresh` method is async, allowing implementations to perform async I/O operations
/// (network requests, database queries, etc.) without blocking the UI.
#[async_trait]
pub trait DataSource: Send + Sync {
    /// The type of data row
    type Row: Send + Sync;

    /// Total number of rows (logical length)
    fn len(&self) -> usize;

    /// Access a row by index (0-based). Behind the scenes this may fetch a page.
    fn get(&self, index: usize) -> Option<&Self::Row>;

    /// Refresh underlying data (may fetch from network, disk, etc.)
    ///
    /// This method is async, allowing implementations to use `.await` for async operations.
    /// The framework will call this method and handle the async execution without blocking the UI.
    async fn refresh(&mut self, ctx: &dyn AppContext) -> Result<()>;
}
