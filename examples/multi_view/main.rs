//! Multi-view example: multiple views with numeric key mapping
//!
//! This example demonstrates how to register multiple views and map them
//! to numeric keys (1-9) for navigation.
//!
//! **Note**: As of version 0.2.0, this framework is async and requires Tokio.

use anyhow::Result;
use async_trait::async_trait;
use crossterm::event::Event;
use ratatui::layout::Rect;
use ratatui::Frame;
use tui_framework::data_source::DataSource;
use tui_framework::keymap::ViewSlot;
use tui_framework::prelude::*;
use tui_framework::view::{HelpItem, Theme, View, ViewResult};
use tui_framework::widget::GridView;

// Simple data row
#[derive(Clone, Debug)]
struct Item {
    id: u32,
    name: String,
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
        // Simulate async data refresh
        Ok(())
    }
}

// View 1
struct View1 {
    grid: GridView<SimpleDataSource>,
}

impl View1 {
    fn new() -> Self {
        let items = vec![
            Item {
                id: 1,
                name: "Item A".to_string(),
            },
            Item {
                id: 2,
                name: "Item B".to_string(),
            },
        ];
        let data_source = SimpleDataSource { items };
        let theme = Theme::default();
        let grid = GridView::new(data_source, theme)
            .with_formatter(|item: &Item| vec![format!("ID: {}", item.id), item.name.clone()]);
        Self { grid }
    }
}

#[async_trait]
impl View for View1 {
    fn id(&self) -> &'static str {
        "view1"
    }

    fn title(&self) -> &'static str {
        "View 1"
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
                key: "1".to_string(),
                description: "Switch to View 1".to_string(),
            },
            HelpItem {
                key: "2".to_string(),
                description: "Switch to View 2".to_string(),
            },
        ]
    }
}

// View 2
struct View2 {
    grid: GridView<SimpleDataSource>,
}

impl View2 {
    fn new() -> Self {
        let items = vec![
            Item {
                id: 3,
                name: "Item C".to_string(),
            },
            Item {
                id: 4,
                name: "Item D".to_string(),
            },
        ];
        let data_source = SimpleDataSource { items };
        let theme = Theme::default();
        let grid = GridView::new(data_source, theme)
            .with_formatter(|item: &Item| vec![format!("ID: {}", item.id), item.name.clone()]);
        Self { grid }
    }
}

#[async_trait]
impl View for View2 {
    fn id(&self) -> &'static str {
        "view2"
    }

    fn title(&self) -> &'static str {
        "View 2"
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
                key: "1".to_string(),
                description: "Switch to View 1".to_string(),
            },
            HelpItem {
                key: "2".to_string(),
                description: "Switch to View 2".to_string(),
            },
        ]
    }
}

// App context
struct MultiViewContext;

impl AppContext for MultiViewContext {}

#[tokio::main]
async fn main() -> Result<()> {
    let mut builder = AppBuilder::new();

    // Register views
    builder = builder
        .register_view(View1::new())
        .register_view(View2::new());

    // Map views to numeric keys
    builder = builder
        .map_view_slot(ViewSlot::Slot1, "view1")
        .map_view_slot(ViewSlot::Slot2, "view2");

    let ctx = MultiViewContext;
    let mut app = builder.build(ctx)?;

    // T062: App::run() is now async
    app.run().await?;

    Ok(())
}
