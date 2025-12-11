//! Example with commands: demonstrates command palette and keybindings
//!
//! This example shows:
//! - Multiple views
//! - Command registration and execution
//! - Command palette (press `:` to open)
//! - Keybindings for view switching
//! - Modal dialogs for command feedback
//!
//! **Note**: As of version 0.2.0, this framework is async and requires Tokio.

use anyhow::{anyhow, Result};
use async_trait::async_trait;
use crossterm::event::Event;
use ratatui::layout::Rect;
use ratatui::Frame;
use std::future::Future;
use std::pin::Pin;
use tui_framework::command::{Command, CommandArgs};
use tui_framework::data_source::DataSource;
use tui_framework::keymap::{KeymapConfig, ViewSlot};
use tui_framework::prelude::*;
use tui_framework::view::{HelpItem, Theme, View, ViewResult};
use tui_framework::widget::GridView;

// Data models
#[derive(Clone, Debug)]
struct Service {
    #[allow(dead_code)]
    id: u32,
    name: String,
    status: String,
    port: u16,
}

// Data source for services
struct ServiceDataSource {
    services: Vec<Service>,
}

#[async_trait]
impl DataSource for ServiceDataSource {
    type Row = Service;

    fn len(&self) -> usize {
        self.services.len()
    }

    fn get(&self, index: usize) -> Option<&Self::Row> {
        self.services.get(index)
    }

    async fn refresh(&mut self, _ctx: &dyn AppContext) -> Result<()> {
        // Simulate async data refresh
        // In a real app: tokio::time::sleep(Duration::from_millis(100)).await;
        Ok(())
    }
}

// Services view
struct ServicesView {
    grid: GridView<ServiceDataSource>,
}

impl ServicesView {
    fn new() -> Self {
        let services = vec![
            Service {
                id: 1,
                name: "web-server".to_string(),
                status: "running".to_string(),
                port: 8080,
            },
            Service {
                id: 2,
                name: "api-server".to_string(),
                status: "stopped".to_string(),
                port: 9090,
            },
            Service {
                id: 3,
                name: "db-server".to_string(),
                status: "running".to_string(),
                port: 5432,
            },
        ];
        let data_source = ServiceDataSource { services };
        let theme = Theme::default();
        let grid = GridView::new(data_source, theme).with_formatter(|service: &Service| {
            vec![
                service.name.clone(),
                service.status.clone(),
                format!(":{}", service.port),
            ]
        });
        Self { grid }
    }
}

#[async_trait]
impl View for ServicesView {
    fn id(&self) -> &'static str {
        "services.view"
    }

    fn title(&self) -> &'static str {
        "Services"
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
                description: "Switch to Services view".to_string(),
            },
            HelpItem {
                key: "2".to_string(),
                description: "Switch to Logs view".to_string(),
            },
            HelpItem {
                key: ":".to_string(),
                description: "Open command palette".to_string(),
            },
            HelpItem {
                key: "?".to_string(),
                description: "Toggle help".to_string(),
            },
        ]
    }

    fn header_info(&self) -> Option<Vec<(String, String)>> {
        Some(vec![
            ("Services".to_string(), "3".to_string()),
            ("Status".to_string(), "Active".to_string()),
        ])
    }

    fn header_help(&self) -> Option<Vec<HelpItem>> {
        Some(vec![
            HelpItem {
                key: ":".to_string(),
                description: "Command palette".to_string(),
            },
            HelpItem {
                key: "?".to_string(),
                description: "Help".to_string(),
            },
        ])
    }
}

// Logs view (placeholder)
struct LogsView;

#[async_trait]
impl View for LogsView {
    fn id(&self) -> &'static str {
        "logs.view"
    }

    fn title(&self) -> &'static str {
        "Logs"
    }

    fn render(&mut self, f: &mut Frame, area: Rect, _ctx: &dyn AppContext) {
        use ratatui::text::Line;
        use ratatui::widgets::{Block, Borders, Paragraph};

        let block = Block::default().borders(Borders::ALL).title("Logs View");

        let text = vec![
            Line::from("This is the logs view."),
            Line::from(""),
            Line::from("Press 1 to switch to Services view."),
            Line::from("Press : to open command palette."),
        ];

        let paragraph = Paragraph::new(text).block(block);
        f.render_widget(paragraph, area);
    }

    async fn handle_event(&mut self, _event: &Event, _ctx: &mut dyn AppContext) -> ViewResult {
        ViewResult::Ignored
    }

    fn help_items(&self) -> Vec<HelpItem> {
        vec![
            HelpItem {
                key: "1".to_string(),
                description: "Switch to Services view".to_string(),
            },
            HelpItem {
                key: "2".to_string(),
                description: "Switch to Logs view".to_string(),
            },
        ]
    }
}

// App context with state
struct MyAppContext {
    #[allow(dead_code)]
    service_count: u32,
    #[allow(dead_code)]
    last_command: Option<String>,
}

impl tui_framework::app::context::AppContext for MyAppContext {
    // AppContext is a marker trait for now
}

// Command implementations (async)
fn restart_service(
    _ctx: &mut dyn AppContext,
    args: CommandArgs,
) -> Pin<Box<dyn Future<Output = Result<()>> + Send>> {
    Box::pin(async move {
        let _service_name = args
            .positional
            .first()
            .map(|s| s.as_str())
            .unwrap_or("default-service");

        // Simulate async command execution
        tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

        Ok(())
    })
}

fn stop_service(
    _ctx: &mut dyn AppContext,
    args: CommandArgs,
) -> Pin<Box<dyn Future<Output = Result<()>> + Send>> {
    Box::pin(async move {
        let _service_name = args
            .positional
            .first()
            .map(|s| s.as_str())
            .unwrap_or("default-service");

        // Simulate async command execution
        tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

        Ok(())
    })
}

fn show_info(
    _ctx: &mut dyn AppContext,
    _args: CommandArgs,
) -> Pin<Box<dyn Future<Output = Result<()>> + Send>> {
    Box::pin(async move {
        // This command will show information
        // The result will be displayed in a modal
        Ok(())
    })
}

fn failing_command(
    _ctx: &mut dyn AppContext,
    _args: CommandArgs,
) -> Pin<Box<dyn Future<Output = Result<()>> + Send>> {
    Box::pin(async move {
        // This command intentionally fails to demonstrate error handling
        Err(anyhow!(
            "This command failed intentionally to demonstrate error handling"
        ))
    })
}

#[tokio::main]
async fn main() -> Result<()> {
    // Create app context
    let ctx = MyAppContext {
        service_count: 3,
        last_command: None,
    };

    // Build application with views
    let mut builder = AppBuilder::new();

    // Register views
    builder = builder
        .register_view(ServicesView::new())
        .register_view(LogsView);

    // Map views to numeric keys
    builder = builder
        .map_view_slot(ViewSlot::Slot1, "services.view")
        .map_view_slot(ViewSlot::Slot2, "logs.view");

    // Register commands (async)
    builder = builder
        .register_command(Command {
            id: "restart",
            summary: "Restart a service",
            syntax: Some("restart <service-name>"),
            category: Some("services"),
            execute: restart_service,
        })
        .register_command(Command {
            id: "stop",
            summary: "Stop a service",
            syntax: Some("stop <service-name>"),
            category: Some("services"),
            execute: stop_service,
        })
        .register_command(Command {
            id: "info",
            summary: "Show application information",
            syntax: None,
            category: Some("system"),
            execute: show_info,
        })
        .register_command(Command {
            id: "fail",
            summary: "Demonstrate error handling (intentionally fails)",
            syntax: None,
            category: Some("demo"),
            execute: failing_command,
        });

    // Configure keymap (optional - demonstrates keybinding configuration)
    let keymap_config = KeymapConfig::new();
    builder = builder.configure_keymap(keymap_config);

    // Build and run
    let mut app = builder.build(ctx)?;

    println!("TUI Framework - Commands Example");
    println!("================================");
    println!("Press ':' to open command palette");
    println!("Press '?' to toggle help");
    println!("Press '1' to switch to Services view");
    println!("Press '2' to switch to Logs view");
    println!("Press 'q' to quit");

    // T062: App::run() is now async
    app.run().await?;

    Ok(())
}
