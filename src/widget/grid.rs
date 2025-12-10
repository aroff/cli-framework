//! GridView widget implementation
//!
//! Displays tabular data using the DataSource trait

use crate::data_source::DataSource;
use crate::view::Theme;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::Span;
use ratatui::widgets::{Block, Borders, Row, Table, TableState};
use ratatui::Frame;
use std::fmt::Debug;

/// GridView widget for displaying tabular data
pub struct GridView<D: DataSource>
where
    D::Row: Debug,
{
    data_source: D,
    state: TableState,
    theme: Theme,
    // T061: Formatter must be Send + Sync for async compatibility
    formatter: Option<Box<dyn Fn(&D::Row) -> Vec<String> + Send + Sync>>,
}

impl<D: DataSource> GridView<D>
where
    D::Row: Debug,
{
    /// Create a new GridView
    pub fn new(data_source: D, theme: Theme) -> Self {
        Self {
            data_source,
            state: TableState::default(),
            theme,
            formatter: None,
        }
    }

    /// Set a custom formatter function for rows
    /// The formatter should return a Vec<String> representing the columns
    /// T061: Formatter must be Send + Sync for async compatibility
    pub fn with_formatter<F>(mut self, formatter: F) -> Self
    where
        F: Fn(&D::Row) -> Vec<String> + Send + Sync + 'static,
    {
        self.formatter = Some(Box::new(formatter));
        self
    }

    /// Get a reference to the data source
    pub fn data_source(&self) -> &D {
        &self.data_source
    }

    /// Get a mutable reference to the data source
    pub fn data_source_mut(&mut self) -> &mut D {
        &mut self.data_source
    }

    /// Get the current selection index
    pub fn selected(&self) -> Option<usize> {
        self.state.selected()
    }

    /// Select a row by index
    pub fn select(&mut self, index: Option<usize>) {
        self.state.select(index);
    }

    /// Move selection down
    pub fn next(&mut self) {
        let len = self.data_source.len();
        if len == 0 {
            return;
        }
        let i = self.state.selected().map_or(0, |i| (i + 1) % len);
        self.state.select(Some(i));
    }

    /// Move selection up
    pub fn previous(&mut self) {
        let len = self.data_source.len();
        if len == 0 {
            return;
        }
        let i = self
            .state
            .selected()
            .map_or(0, |i| if i == 0 { len - 1 } else { i - 1 });
        self.state.select(Some(i));
    }

    /// Render the grid view
    pub fn render(&mut self, f: &mut Frame, area: Rect) {
        let len = self.data_source.len();

        if len == 0 {
            // Show empty state
            let block = Block::default().borders(Borders::ALL).title("No Data");
            f.render_widget(block, area);
            return;
        }

        // Build rows from data source
        let mut rows = Vec::new();
        let visible_rows = (area.height.saturating_sub(2)) as usize;
        let mut num_cols = 1;

        for i in 0..len.min(visible_rows) {
            if let Some(row) = self.data_source.get(i) {
                let cells = if let Some(ref formatter) = self.formatter {
                    // Use custom formatter
                    formatter(row)
                } else {
                    // Default: use Debug formatting, try to parse as structured data
                    let debug_str = format!("{:?}", row);
                    // Try to extract meaningful fields from Debug output
                    // For structs like Item { id: 1, name: "...", status: "..." }
                    // we'll show a simplified version
                    if debug_str.contains("id:") && debug_str.contains("name:") {
                        // Try to extract name field if it exists
                        let name = if let Some(start) = debug_str.find("name: \"") {
                            let start = start + 7;
                            if let Some(end) = debug_str[start..].find('"') {
                                debug_str[start..start + end].to_string()
                            } else {
                                format!("Item {}", i + 1)
                            }
                        } else {
                            format!("Item {}", i + 1)
                        };
                        vec![name]
                    } else {
                        // Fallback to Debug string (truncated)
                        vec![debug_str.chars().take(area.width as usize - 4).collect()]
                    }
                };

                num_cols = num_cols.max(cells.len());
                let spans: Vec<Span> = cells.iter().map(|s| Span::raw(s.clone())).collect();
                rows.push(Row::new(spans));
            }
        }

        // Create column constraints (equal width for all columns)
        let widths: Vec<ratatui::layout::Constraint> = (0..num_cols)
            .map(|_| ratatui::layout::Constraint::Percentage(100 / num_cols.max(1) as u16))
            .collect();

        let table = Table::new(rows, widths)
            .block(Block::default().borders(Borders::ALL).title("Data"))
            .highlight_style(
                Style::default().add_modifier(Modifier::REVERSED).fg(self
                    .theme
                    .primary_style
                    .fg
                    .unwrap_or(ratatui::style::Color::Cyan)),
            );

        f.render_stateful_widget(table, area, &mut self.state);
    }
}
