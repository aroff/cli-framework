//! Simple example: one view, one datasource
//!
//! This example demonstrates the minimal setup required to create a TUI
//! with a single grid view displaying data from a datasource.

use tui_framework::prelude::*;
use tui_framework::data_source::DataSource;
use tui_framework::view::{View, ViewResult, HelpItem, Theme};
use tui_framework::widget::GridView;
use tui_framework::message::AppMessage;
use anyhow::Result;
use crossterm::event::Event;
use ratatui::layout::Rect;
use ratatui::Frame;

// Simple data row
#[derive(Clone, Debug)]
struct Item {
    id: u32,
    name: String,
    status: String,
}

// Simple in-memory data source
struct SimpleDataSource {
    items: Vec<Item>,
}

impl DataSource for SimpleDataSource {
    type Row = Item;

    fn len(&self) -> usize {
        self.items.len()
    }

    fn get(&self, index: usize) -> Option<&Self::Row> {
        self.items.get(index)
    }

    fn refresh(&mut self, _ctx: &dyn AppContext) -> Result<()> {
        // In a real application, this would fetch data from a service
        // For this example, we'll just keep the existing data
        Ok(())
    }
}

// Simple view that displays a grid
struct SimpleView {
    grid: GridView<SimpleDataSource>,
}

impl SimpleView {
    fn new() -> Self {
        let items = vec![
            Item { id: 1, name: "Item 1".to_string(), status: "Active".to_string() },
            Item { id: 2, name: "Item 2".to_string(), status: "Inactive".to_string() },
            Item { id: 3, name: "Item 3".to_string(), status: "Active".to_string() },
        ];
        let data_source = SimpleDataSource { items };
        let theme = Theme::default();
        let grid = GridView::new(data_source, theme)
            .with_formatter(|item: &Item| {
                vec![
                    format!("ID: {}", item.id),
                    item.name.clone(),
                    item.status.clone(),
                ]
            });
        Self { grid }
    }
}

impl View for SimpleView {
    fn id(&self) -> &'static str {
        "simple.view"
    }

    fn title(&self) -> &'static str {
        "Simple View"
    }

    fn render(&mut self, f: &mut Frame, area: Rect, _ctx: &dyn AppContext) {
        self.grid.render(f, area);
    }

    fn handle_event(&mut self, _event: &Event, _ctx: &mut dyn AppContext) -> ViewResult {
        ViewResult::Ignored
    }

    fn help_items(&self) -> Vec<HelpItem> {
        vec![
            HelpItem {
                key: "q".to_string(),
                description: "Quit".to_string(),
            },
            HelpItem {
                key: "?".to_string(),
                description: "Toggle help".to_string(),
            },
        ]
    }
}

// Simple app context
struct SimpleContext;

impl AppContext for SimpleContext {}

fn main() -> Result<()> {
    let mut builder = AppBuilder::new();
    builder = builder.register_view(SimpleView::new());
    
    let ctx = SimpleContext;
    let mut app = builder.build(ctx)?;
    
    app.run()?;
    
    Ok(())
}
