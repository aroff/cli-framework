//! DataSource trait definition
//!
//! Defines the DataSource trait for providing tabular data to GridView.

use crate::app::context::AppContext;
use anyhow::Result;

/// DataSource trait for providing tabular data
pub trait DataSource {
    /// The type of data row
    type Row;

    /// Total number of rows (logical length)
    fn len(&self) -> usize;

    /// Access a row by index (0-based). Behind the scenes this may fetch a page.
    fn get(&self, index: usize) -> Option<&Self::Row>;

    /// Refresh underlying data (may fetch from network, disk, etc.)
    fn refresh(&mut self, ctx: &dyn AppContext) -> Result<()>;
}

