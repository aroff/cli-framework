//! Table formatting utilities
//!
//! Provides functions for formatting grid-like data as readable tables.

use anyhow::{Context, Result};
use serde::Serialize;

/// Column alignment options
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Alignment {
    /// Left-align column content
    #[default]
    Left,
    /// Right-align column content
    Right,
    /// Center-align column content
    Center,
}

/// Column definition metadata
#[derive(Debug, Clone, Default)]
pub struct ColumnDef {
    /// Column header name
    pub name: String,
    /// Optional width hint for column
    pub width_hint: Option<usize>,
    /// Optional alignment (default: Left)
    pub alignment: Option<Alignment>,
}

/// Grid data structure with metadata
///
/// Represents grid-like data with column definitions and optional row headers.
/// The data type T should be a flat struct with primitive fields (string, number, boolean).
#[derive(Debug, Clone)]
pub struct GridData<T> {
    /// Data rows
    pub rows: Vec<T>,
    /// Column metadata definitions
    pub columns: Vec<ColumnDef>,
    /// Optional row headers
    pub row_headers: Option<Vec<String>>,
}

impl<T> GridData<T> {
    /// Validate GridData structure
    ///
    /// Ensures that:
    /// - Columns are not empty
    /// - If row_headers is Some, its length matches rows.len()
    pub fn validate(&self) -> Result<()> {
        if self.columns.is_empty() {
            return Err(anyhow::anyhow!("GridData must have at least one column"));
        }

        if let Some(ref headers) = self.row_headers {
            if headers.len() != self.rows.len() {
                return Err(anyhow::anyhow!(
                    "Row headers length ({}) must match rows length ({})",
                    headers.len(),
                    self.rows.len()
                ));
            }
        }

        for col in &self.columns {
            if col.name.is_empty() {
                return Err(anyhow::anyhow!("Column name cannot be empty"));
            }
            if let Some(width) = col.width_hint {
                if width == 0 {
                    return Err(anyhow::anyhow!("Column width_hint must be > 0"));
                }
            }
        }

        Ok(())
    }
}

/// Format grid data as a readable table
///
/// Formats grid-like data as a table with column headers and aligned rows.
/// Supports multi-line values, missing/null values, and different output modes.
///
/// # Arguments
///
/// * `grid` - Grid data with rows and column metadata
///
/// # Returns
///
/// Formatted table string with column headers and aligned rows
///
/// # Example
///
/// ```rust,no_run
/// use cli_framework::cli_output::{GridData, ColumnDef, Alignment, format_table};
/// use serde::Serialize;
///
/// #[derive(Serialize)]
/// struct User {
///     name: String,
///     email: String,
/// }
///
/// let grid = GridData {
///     rows: vec![
///         User { name: "Alice".to_string(), email: "alice@example.com".to_string() },
///     ],
///     columns: vec![
///         ColumnDef { name: "Name".to_string(), width_hint: None, alignment: Some(Alignment::Left) },
///         ColumnDef { name: "Email".to_string(), width_hint: None, alignment: Some(Alignment::Left) },
///     ],
///     row_headers: None,
/// };
///
/// let table = format_table(&grid)?;
/// println!("{}", table);
/// # Ok::<(), anyhow::Error>(())
/// ```
pub fn format_table<T: Serialize>(grid: &GridData<T>) -> Result<String> {
    grid.validate().context("Invalid grid data")?;

    // Handle empty data
    if grid.rows.is_empty() {
        return Ok("(empty)".to_string());
    }

    // For now, implement basic table formatting
    // This is a simplified version - full implementation will handle:
    // - Column width calculation
    // - Alignment
    // - Multi-line values
    // - TUI/CLI mode differences
    // - Missing/null values

    let mut result = String::new();

    // Calculate column widths
    let mut col_widths: Vec<usize> = grid
        .columns
        .iter()
        .map(|col| col.width_hint.unwrap_or(0).max(col.name.len()))
        .collect();

    // Calculate actual widths from data
    for row in &grid.rows {
        let row_data = serde_json::to_value(row).context("Failed to serialize row data")?;

        if let serde_json::Value::Object(map) = row_data {
            for (i, col) in grid.columns.iter().enumerate() {
                if let Some(value) = map
                    .iter()
                    .find(|(k, _)| k.eq_ignore_ascii_case(&col.name))
                    .map(|(_, v)| v)
                    .or_else(|| map.get(&col.name))
                {
                    let value_str = value_to_string(value);
                    // Handle multi-line values - use first line for width calculation
                    let first_line_len = value_str.lines().next().map(|l| l.len()).unwrap_or(0);
                    col_widths[i] = col_widths[i].max(first_line_len);
                }
            }
        }
    }

    // Header row
    let header: Vec<String> = grid.columns.iter().map(|c| c.name.clone()).collect();
    result.push_str(&format_row_with_widths(&header, &grid.columns, &col_widths));
    result.push('\n');

    // Separator
    let separator: Vec<String> = col_widths.iter().map(|w| "-".repeat(*w)).collect();
    result.push_str(&format_row_with_widths(
        &separator,
        &grid.columns,
        &col_widths,
    ));
    result.push('\n');

    // Data rows - extract field values from serialized data
    for row in &grid.rows {
        let row_data = serde_json::to_value(row).context("Failed to serialize row data")?;

        let row_values: Vec<String> = if let serde_json::Value::Object(map) = row_data {
            grid.columns
                .iter()
                .map(|col| {
                    // Try to find matching field (case-insensitive)
                    let value = map
                        .iter()
                        .find(|(k, _)| k.eq_ignore_ascii_case(&col.name))
                        .map(|(_, v)| value_to_string(v))
                        .unwrap_or_else(|| {
                            // If not found, try exact match
                            map.get(&col.name).map(value_to_string).unwrap_or_else(|| {
                                // Handle missing/null values (FR-001b)
                                String::new()
                            })
                        });
                    value
                })
                .collect()
        } else {
            vec!["".to_string(); grid.columns.len()]
        };

        // Handle multi-line values (FR-011a)
        let multi_line_rows = format_multi_line_row(&row_values, &grid.columns, &col_widths);
        for line in multi_line_rows {
            result.push_str(&line);
            result.push('\n');
        }
    }

    Ok(result)
}

/// Convert a JSON value to a string representation
fn value_to_string(value: &serde_json::Value) -> String {
    match value {
        serde_json::Value::String(s) => s.clone(),
        serde_json::Value::Number(n) => n.to_string(),
        serde_json::Value::Bool(b) => b.to_string(),
        serde_json::Value::Null => String::new(), // Handle null values (FR-001b)
        serde_json::Value::Array(_) | serde_json::Value::Object(_) => {
            // For nested structures, use JSON representation
            serde_json::to_string(value).unwrap_or_else(|_| String::new())
        }
    }
}

/// Format a single row with specified widths
fn format_row_with_widths(values: &[String], columns: &[ColumnDef], widths: &[usize]) -> String {
    let mut parts = Vec::new();
    let default_col = ColumnDef::default();

    for (i, value) in values.iter().enumerate() {
        let col = columns.get(i).unwrap_or(&default_col);
        let width = widths.get(i).copied().unwrap_or(20);

        // Truncate if needed (for TUI mode - will be enhanced later)
        let display_value = if value.len() > width {
            &value[..width.min(value.len())]
        } else {
            value
        };

        let aligned = match col.alignment.unwrap_or(Alignment::Left) {
            Alignment::Left => format!("{:<width$}", display_value, width = width),
            Alignment::Right => format!("{:>width$}", display_value, width = width),
            Alignment::Center => {
                let padding = width.saturating_sub(display_value.len());
                let left_pad = padding / 2;
                let right_pad = padding - left_pad;
                format!(
                    "{}{}{}",
                    " ".repeat(left_pad),
                    display_value,
                    " ".repeat(right_pad)
                )
            }
        };

        parts.push(aligned);
    }

    parts.join("  ")
}

/// Format a row with multi-line values (FR-011a)
/// Returns multiple lines if any cell contains newlines
fn format_multi_line_row(
    values: &[String],
    columns: &[ColumnDef],
    widths: &[usize],
) -> Vec<String> {
    // Split each value by newlines
    let cell_lines: Vec<Vec<&str>> = values.iter().map(|v| v.lines().collect()).collect();

    // Find maximum number of lines
    let max_lines = cell_lines
        .iter()
        .map(|lines| lines.len())
        .max()
        .unwrap_or(1);

    let mut result = Vec::new();
    for line_idx in 0..max_lines {
        let line_values: Vec<String> = cell_lines
            .iter()
            .map(|lines| {
                lines
                    .get(line_idx)
                    .map(|s| s.to_string())
                    .unwrap_or_default()
            })
            .collect();
        result.push(format_row_with_widths(&line_values, columns, widths));
    }

    result
}

/// Print grid data as a table to stdout
///
/// # Arguments
///
/// * `grid` - Grid data with rows and column metadata
pub fn print_table<T: Serialize>(grid: &GridData<T>) -> Result<()> {
    let table = format_table(grid)?;
    print!("{}", table);
    Ok(())
}
