pub mod registry;
pub mod parser;
pub mod palette;

pub use registry::CommandRegistry;
pub use palette::{CommandPalette, CommandPaletteResult};

use crate::app::context::AppContext;
use anyhow::Result;
use std::collections::HashMap;

/// Command identifier
pub type CommandId = &'static str;

/// Command arguments (positional and named)
#[derive(Debug, Clone)]
pub struct CommandArgs {
    /// Positional arguments
    pub positional: Vec<String>,
    /// Named arguments (key-value pairs)
    pub named: HashMap<String, String>,
}

/// Command result type
pub type CommandResult = Result<()>;

/// Command struct representing an executable operation
#[derive(Debug, Clone)]
pub struct Command {
    /// Unique command identifier
    pub id: CommandId,
    /// Short description (shown in command palette)
    pub summary: &'static str,
    /// Optional syntax hint (e.g., ":restart service=<name> env=<env>")
    pub syntax: Option<&'static str>,
    /// Optional category for grouping in palette
    pub category: Option<&'static str>,
    /// Execution function
    pub execute: fn(&mut dyn AppContext, CommandArgs) -> CommandResult,
}
