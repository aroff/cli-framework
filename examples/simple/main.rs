//! Simple example: one view, one datasource
//!
//! This example demonstrates the minimal setup required to create a TUI
//! with a single grid view displaying data from a datasource.
//!
//! **Note**: As of version 0.2.0, this framework is async and requires Tokio.
//! All DataSource and View implementations must use async methods.

use anyhow::Result;
use async_trait::async_trait;
use crossterm::event::Event;
use ratatui::layout::Rect;
use ratatui::Frame;
use tui_framework::data_source::DataSource;
use tui_framework::message::AppMessage;
use tui_framework::prelude::*;
use tui_framework::view::{HelpItem, Theme, View, ViewResult};
use tui_framework::widget::GridView;

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

#[async_trait]
impl DataSource for SimpleDataSource {
    type Row = Item;

    fn len(&self) -> usize {
        self.items.len()
    }

    fn get(&self, index: usize) -> Option<&Self::Row> {
        self.items.get(index)
    }

    async fn refresh(&mut self, _ctx: &dyn AppContext) -> Result<()> {
        // In a real application, this would fetch data from a service using async I/O
        // For this example, we'll just keep the existing data
        // Example: tokio::time::sleep(Duration::from_millis(100)).await;
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
            Item {
                id: 1,
                name: "Item 1".to_string(),
                status: "Active".to_string(),
            },
            Item {
                id: 2,
                name: "Item 2".to_string(),
                status: "Inactive".to_string(),
            },
            Item {
                id: 3,
                name: "Item 3".to_string(),
                status: "Active".to_string(),
            },
        ];
        let data_source = SimpleDataSource { items };
        let theme = Theme::default();
        // Formatter must be Send + Sync for async compatibility
        let grid = GridView::new(data_source, theme).with_formatter(|item: &Item| -> Vec<String> {
            vec![
                format!("ID: {}", item.id),
                item.name.clone(),
                item.status.clone(),
            ]
        });
        Self { grid }
    }
}

#[async_trait]
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

    async fn handle_event(&mut self, _event: &Event, _ctx: &mut dyn AppContext) -> ViewResult {
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

#[tokio::main]
async fn main() -> Result<()> {
    let mut builder = AppBuilder::new();
    builder = builder.register_view(SimpleView::new());

    let ctx = SimpleContext;
    let mut app = builder.build(ctx)?;

    // T061: App::run() is now async and must be awaited
    app.run().await?;

    Ok(())
}
