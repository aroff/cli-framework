//! JSON formatting utilities
//!
//! Provides functions for formatting data as JSON in both pretty-printed and compact formats.

use anyhow::{Context, Result};
use serde::Serialize;

/// Format data as pretty-printed JSON with 2-space indentation
///
/// Formats a serializable data structure as human-readable JSON with proper
/// indentation (2 spaces per level).
///
/// # Arguments
///
/// * `data` - Serializable data structure
///
/// # Returns
///
/// Pretty-printed JSON string with 2-space indentation (FR-002a)
///
/// # Example
///
/// ```rust,no_run
/// use cli_framework::cli_output::format_json;
/// use serde::Serialize;
///
/// #[derive(Serialize)]
/// struct Config {
///     host: String,
///     port: u16,
/// }
///
/// let config = Config { host: "localhost".to_string(), port: 8080 };
/// let json = format_json(&config)?;
/// // Output: {
/// //   "host": "localhost",
/// //   "port": 8080
/// // }
/// # Ok::<(), anyhow::Error>(())
/// ```
pub fn format_json<T: Serialize>(data: &T) -> Result<String> {
    serde_json::to_string_pretty(data).context("Failed to serialize to JSON")
}

/// Format data as compact (single-line) JSON
///
/// Formats a serializable data structure as compact JSON suitable for
/// piping to other tools or scripts.
///
/// # Arguments
///
/// * `data` - Serializable data structure
///
/// # Returns
///
/// Compact JSON string (single line) (FR-002)
///
/// # Example
///
/// ```rust,no_run
/// use cli_framework::cli_output::format_json_compact;
/// use serde::Serialize;
///
/// #[derive(Serialize)]
/// struct Config {
///     host: String,
///     port: u16,
/// }
///
/// let config = Config { host: "localhost".to_string(), port: 8080 };
/// let json = format_json_compact(&config)?;
/// // Output: {"host":"localhost","port":8080}
/// # Ok::<(), anyhow::Error>(())
/// ```
pub fn format_json_compact<T: Serialize>(data: &T) -> Result<String> {
    serde_json::to_string(data).context("Failed to serialize to JSON")
}

/// Print data as JSON to stdout
///
/// Formats and prints data as pretty-printed JSON to stdout.
///
/// # Arguments
///
/// * `data` - Serializable data structure
///
/// # Example
///
/// ```rust,no_run
/// use cli_framework::cli_output::print_json;
/// use serde::Serialize;
///
/// #[derive(Serialize)]
/// struct Config {
///     host: String,
///     port: u16,
/// }
///
/// let config = Config { host: "localhost".to_string(), port: 8080 };
/// print_json(&config)?;
/// # Ok::<(), anyhow::Error>(())
/// ```
pub fn print_json<T: Serialize>(data: &T) -> Result<()> {
    let json = format_json(data)?;
    println!("{}", json);
    Ok(())
}
