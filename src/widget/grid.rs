//! GridView widget implementation
//!
//! Displays tabular data using the DataSource trait

use crate::data_source::DataSource;
use crate::view::Theme;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Row, Table, TableState};
use ratatui::Frame;

/// GridView widget for displaying tabular data
pub struct GridView<D: DataSource> {
    data_source: D,
    state: TableState,
    theme: Theme,
}

impl<D: DataSource> GridView<D> {
    /// Create a new GridView
    pub fn new(data_source: D, theme: Theme) -> Self {
        Self {
            data_source,
            state: TableState::default(),
            theme,
        }
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
        let i = self.state.selected().map_or(0, |i| if i == 0 { len - 1 } else { i - 1 });
        self.state.select(Some(i));
    }

    /// Render the grid view
    pub fn render(&mut self, f: &mut Frame, area: Rect) {
        let len = self.data_source.len();
        
        if len == 0 {
            // Show empty state
            let block = Block::default()
                .borders(Borders::ALL)
                .title("No Data");
            f.render_widget(block, area);
            return;
        }

        // Build rows from data source
        // Note: Applications will need to provide a way to format rows
        // For now, we'll create a placeholder implementation
        let mut rows = Vec::new();
        for i in 0..len.min((area.height.saturating_sub(2)) as usize) {
            if self.data_source.get(i).is_some() {
                // Placeholder: applications will customize row formatting
                rows.push(Row::new(vec![Span::raw(format!("Row {}", i + 1))]));
            }
        }

        let widths = [ratatui::layout::Constraint::Percentage(100)];
        let table = Table::new(rows, widths)
            .block(Block::default().borders(Borders::ALL).title("Data"))
            .highlight_style(
                Style::default()
                    .add_modifier(Modifier::REVERSED)
                    .fg(self.theme.primary_style.fg.unwrap_or(ratatui::style::Color::Cyan)),
            );

        f.render_stateful_widget(table, area, &mut self.state);
    }
}
