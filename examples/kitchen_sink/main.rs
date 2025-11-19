//! Kitchen sink example: comprehensive reference application
//!
//! This example implements all framework features (auth, grid, logs, commands)
//! to serve as the primary integration test bench and demonstrate best practices.

use tui_framework::prelude::*;
use tui_framework::data_source::{DataSource, SharedLogBuffer, sync_log_buffer_to_view};
use tui_framework::view::{View, ViewResult, HelpItem, Theme};
use tui_framework::widget::{GridView, LogView};
use tui_framework::message::AppMessage;
use tui_framework::command::{Command, CommandArgs};
use tui_framework::keymap::ViewSlot;
use anyhow::Result;
use crossterm::event::{Event, KeyCode};
use ratatui::layout::Rect;
use ratatui::Frame;
use std::thread;
use std::time::{Duration, Instant};

// Data models
#[derive(Clone)]
struct Resource {
    id: u32,
    name: String,
    status: String,
}

// Data source for resources
struct ResourceDataSource {
    resources: Vec<Resource>,
}

impl DataSource for ResourceDataSource {
    type Row = Resource;

    fn len(&self) -> usize {
        self.resources.len()
    }

    fn get(&self, index: usize) -> Option<&Self::Row> {
        self.resources.get(index)
    }

    fn refresh(&mut self, _ctx: &dyn AppContext) -> Result<()> {
        Ok(())
    }
}

// Resources view with GridView
struct ResourcesView {
    grid: GridView<ResourceDataSource>,
}

impl ResourcesView {
    fn new() -> Self {
        let resources = vec![
            Resource { id: 1, name: "web-server".to_string(), status: "running".to_string() },
            Resource { id: 2, name: "api-server".to_string(), status: "running".to_string() },
            Resource { id: 3, name: "db-server".to_string(), status: "stopped".to_string() },
        ];
        let data_source = ResourceDataSource { resources };
        let theme = Theme::default();
        let grid = GridView::new(data_source, theme);
        Self { grid }
    }
}

impl View for ResourcesView {
    fn id(&self) -> &'static str {
        "resources.view"
    }

    fn title(&self) -> &'static str {
        "Resources"
    }

    fn render(&mut self, f: &mut Frame, area: Rect, _ctx: &dyn AppContext) {
        self.grid.render(f, area);
    }

    fn handle_event(&mut self, event: &Event, _ctx: &mut dyn AppContext) -> ViewResult {
        if let Event::Key(key) = event {
            match key.code {
                KeyCode::Down | KeyCode::Char('j') => {
                    self.grid.next();
                    return ViewResult::Handled;
                }
                KeyCode::Up | KeyCode::Char('k') => {
                    self.grid.previous();
                    return ViewResult::Handled;
                }
                _ => {}
            }
        }
        ViewResult::Ignored
    }

    fn help_items(&self) -> Vec<HelpItem> {
        vec![
            HelpItem {
                key: "j/↓".to_string(),
                description: "Move down".to_string(),
            },
            HelpItem {
                key: "k/↑".to_string(),
                description: "Move up".to_string(),
            },
        ]
    }
}

// Logs view with LogView
struct LogsView {
    log_view: LogView,
    log_buffer: SharedLogBuffer,
    filter_input: String,
    filter_mode: bool,
}

impl LogsView {
    fn new() -> Self {
        let theme = Theme::default();
        let log_view = LogView::new(theme);
        let log_buffer = SharedLogBuffer::new(10000);
        
        // Start a background thread to simulate log streaming
        let buffer_clone = log_buffer.clone();
        thread::spawn(move || {
            let mut counter = 0;
            loop {
                thread::sleep(Duration::from_millis(500));
                counter += 1;
                
                let now = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_secs();
                let timestamp = format!("{:02}:{:02}:{:02}", 
                    (now / 3600) % 24,
                    (now / 60) % 60,
                    now % 60);
                let level = if counter % 10 == 0 {
                    "ERROR"
                } else if counter % 5 == 0 {
                    "WARN"
                } else {
                    "INFO"
                };
                
                let message = format!("[{}] {}: Log message #{}", timestamp, level, counter);
                buffer_clone.push(message);
            }
        });
        
        Self {
            log_view,
            log_buffer,
            filter_input: String::new(),
            filter_mode: false,
        }
    }
}

impl View for LogsView {
    fn id(&self) -> &'static str {
        "logs.view"
    }

    fn title(&self) -> &'static str {
        "Logs"
    }

    fn render(&mut self, f: &mut Frame, area: Rect, _ctx: &dyn AppContext) {
        // Sync log buffer to view
        sync_log_buffer_to_view(&self.log_buffer, &mut self.log_view);
        
        // Render log view
        self.log_view.render(f, area);
    }

    fn handle_event(&mut self, event: &Event, _ctx: &mut dyn AppContext) -> ViewResult {
        if let Event::Key(key) = event {
            match key.code {
                KeyCode::Char('/') => {
                    // Enter filter mode
                    self.filter_mode = true;
                    self.filter_input.clear();
                    return ViewResult::Handled;
                }
                KeyCode::Char('f') => {
                    // Toggle follow mode
                    self.log_view.toggle_follow_mode();
                    return ViewResult::Handled;
                }
                KeyCode::Up | KeyCode::Char('k') => {
                    self.log_view.scroll_up();
                    return ViewResult::Handled;
                }
                KeyCode::Down | KeyCode::Char('j') => {
                    self.log_view.scroll_down();
                    return ViewResult::Handled;
                }
                KeyCode::PageUp => {
                    self.log_view.page_up();
                    return ViewResult::Handled;
                }
                KeyCode::PageDown => {
                    self.log_view.page_down();
                    return ViewResult::Handled;
                }
                KeyCode::Char('g') => {
                    self.log_view.scroll_to_top();
                    return ViewResult::Handled;
                }
                KeyCode::Char('G') => {
                    self.log_view.scroll_to_bottom();
                    return ViewResult::Handled;
                }
                KeyCode::Esc => {
                    if self.filter_mode {
                        self.filter_mode = false;
                        self.log_view.set_filter(None);
                        return ViewResult::Handled;
                    }
                }
                KeyCode::Enter => {
                    if self.filter_mode {
                        self.filter_mode = false;
                        let filter = if self.filter_input.is_empty() {
                            None
                        } else {
                            Some(self.filter_input.clone())
                        };
                        self.log_view.set_filter(filter);
                        return ViewResult::Handled;
                    }
                }
                KeyCode::Backspace => {
                    if self.filter_mode {
                        self.filter_input.pop();
                        return ViewResult::Handled;
                    }
                }
                KeyCode::Char(c) => {
                    if self.filter_mode {
                        self.filter_input.push(c);
                        return ViewResult::Handled;
                    }
                }
                _ => {}
            }
        }
        ViewResult::Ignored
    }

    fn help_items(&self) -> Vec<HelpItem> {
        vec![
            HelpItem {
                key: "/".to_string(),
                description: "Enter filter mode".to_string(),
            },
            HelpItem {
                key: "f".to_string(),
                description: "Toggle follow mode".to_string(),
            },
            HelpItem {
                key: "j/↓".to_string(),
                description: "Scroll down".to_string(),
            },
            HelpItem {
                key: "k/↑".to_string(),
                description: "Scroll up".to_string(),
            },
            HelpItem {
                key: "g".to_string(),
                description: "Scroll to top".to_string(),
            },
            HelpItem {
                key: "G".to_string(),
                description: "Scroll to bottom".to_string(),
            },
        ]
    }
}

// App context
struct KitchenSinkContext;

impl AppContext for KitchenSinkContext {}

// Command implementations
fn clear_logs(_ctx: &mut dyn AppContext, _args: CommandArgs) -> Result<()> {
    // This would clear logs in a real application
    Ok(())
}

fn main() -> Result<()> {
    let ctx = KitchenSinkContext;

    // Build application
    let mut builder = AppBuilder::new();
    
    // Register views
    builder = builder
        .register_view(ResourcesView::new())
        .register_view(LogsView::new());
    
    // Map views to F-keys
    builder = builder
        .map_view_slot(ViewSlot::F1, "resources.view")
        .map_view_slot(ViewSlot::F2, "logs.view");
    
    // Register commands
    builder = builder.register_command(Command {
        id: "clear-logs",
        summary: "Clear log buffer",
        syntax: None,
        category: Some("logs"),
        execute: clear_logs,
    });
    
    // Build and run
    let mut app = builder.build(ctx)?;
    
    println!("TUI Framework - Kitchen Sink Example");
    println!("=====================================");
    println!("This example demonstrates:");
    println!("  - GridView with data sources");
    println!("  - LogView with streaming logs");
    println!("  - Command palette (press ':')");
    println!("  - View switching (F1, F2)");
    println!("  - Help overlay (press '?')");
    println!("");
    println!("Log View Controls:");
    println!("  / - Enter filter mode");
    println!("  f - Toggle follow mode");
    println!("  j/↓ - Scroll down");
    println!("  k/↑ - Scroll up");
    println!("  g - Scroll to top");
    println!("  G - Scroll to bottom");
    println!("");
    
    app.run()?;
    
    Ok(())
}
