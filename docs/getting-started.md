# Getting Started: Creating Your First TUI Application

This tutorial will guide you through creating your first TUI application using the TUI Framework. By the end, you'll have a working application that displays a list of servers with the ability to refresh data and execute commands.

## Prerequisites

- Rust toolchain (latest stable, minimum 1.70+)
- Basic familiarity with Rust
- Terminal with at least 80x24 characters

## Step 1: Create a New Project

Create a new Rust project:

```bash
cargo new my-tui-app
cd my-tui-app
```

## Step 2: Add Dependencies

Add the following to your `Cargo.toml`:

```toml
[dependencies]
tui-framework = { path = "../tui-framework" }  # Adjust path as needed
anyhow = "1.0"
ratatui = "0.27"
crossterm = "0.28"
```

## Step 3: Define Your Data Model

Create `src/main.rs` and start by defining your data model:

```rust
use tui_framework::prelude::*;
use tui_framework::data_source::DataSource;
use tui_framework::view::{View, ViewResult, HelpItem, Theme};
use tui_framework::widget::GridView;
use tui_framework::command::{Command, CommandArgs};
use anyhow::Result;
use crossterm::event::{Event, KeyCode};
use ratatui::layout::Rect;
use ratatui::Frame;

// Your application's data model
#[derive(Clone, Debug)]
struct Server {
    id: String,
    name: String,
    status: String,
    port: u16,
}

// Simple in-memory data source
struct ServersDataSource {
    servers: Vec<Server>,
}

impl DataSource for ServersDataSource {
    type Row = Server;

    fn len(&self) -> usize {
        self.servers.len()
    }

    fn get(&self, index: usize) -> Option<&Self::Row> {
        self.servers.get(index)
    }

    fn refresh(&mut self, _ctx: &dyn AppContext) -> Result<()> {
        // In a real application, this would fetch data from a service
        // For this tutorial, we'll simulate with static data
        self.servers = vec![
            Server {
                id: "1".to_string(),
                name: "api-server-1".to_string(),
                status: "running".to_string(),
                port: 8080,
            },
            Server {
                id: "2".to_string(),
                name: "api-server-2".to_string(),
                status: "stopped".to_string(),
                port: 8081,
            },
            Server {
                id: "3".to_string(),
                name: "web-server-1".to_string(),
                status: "running".to_string(),
                port: 9090,
            },
        ];
        Ok(())
    }
}
```

## Step 4: Create a View

Now create a view that displays your servers:

```rust
struct ServersView {
    grid: GridView<ServersDataSource>,
}

impl ServersView {
    fn new() -> Self {
        let data_source = ServersDataSource {
            servers: Vec::new(),
        };
        let theme = Theme::default();
        let grid = GridView::new(data_source, theme)
            .with_formatter(|server: &Server| {
                vec![
                    server.id.clone(),
                    server.name.clone(),
                    server.status.clone(),
                    format!(":{}", server.port),
                ]
            });
        Self { grid }
    }
}

impl View for ServersView {
    fn id(&self) -> &'static str {
        "servers"
    }

    fn title(&self) -> &'static str {
        "Servers"
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
                KeyCode::Char('r') => {
                    // Refresh data
                    if let Err(e) = self.grid.data_source_mut().refresh(_ctx) {
                        return ViewResult::ShowModal(
                            tui_framework::message::AppMessage::error(
                                "Refresh failed".to_string(),
                                Some(e.to_string()),
                            ),
                        );
                    }
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
            HelpItem {
                key: "r".to_string(),
                description: "Refresh servers".to_string(),
            },
        ]
    }

    fn header_info(&self) -> Option<Vec<(String, String)>> {
        Some(vec![
            ("Servers".to_string(), format!("{}", self.grid.data_source().len())),
            ("Status".to_string(), "Active".to_string()),
        ])
    }

    fn header_help(&self) -> Option<Vec<HelpItem>> {
        Some(vec![
            HelpItem {
                key: "r".to_string(),
                description: "Refresh".to_string(),
            },
            HelpItem {
                key: ":".to_string(),
                description: "Commands".to_string(),
            },
            HelpItem {
                key: "?".to_string(),
                description: "Help".to_string(),
            },
        ])
    }
}
```

## Step 5: Define AppContext

Create a simple application context:

```rust
struct MyAppContext;

impl AppContext for MyAppContext {
    // AppContext is a marker trait for now
}
```

## Step 6: Add Commands (Optional)

Add a command to restart a server:

```rust
fn restart_server(_ctx: &mut dyn AppContext, args: CommandArgs) -> Result<()> {
    let server_id = args.positional.get(0)
        .ok_or_else(|| anyhow::anyhow!("Missing server ID"))?;
    
    // In a real application, this would call your service API
    println!("Restarting server: {}", server_id);
    
    Ok(())
}

fn refresh_servers(_ctx: &mut dyn AppContext, _args: CommandArgs) -> Result<()> {
    // This would trigger a refresh in a real application
    println!("Refreshing servers...");
    Ok(())
}
```

## Step 7: Build and Run

Put it all together in `main()`:

```rust
fn main() -> Result<()> {
    let ctx = MyAppContext;

    // Build the application
    let mut builder = AppBuilder::new();
    
    // Register the view
    builder = builder.register_view(ServersView::new());
    
    // Map view to numeric key 1
    builder = builder.map_view_slot(ViewSlot::Slot1, "servers");
    
    // Register commands
    builder = builder
        .register_command(Command {
            id: "restart",
            summary: "Restart a server",
            syntax: Some("restart <server-id>"),
            category: Some("servers"),
            execute: restart_server,
        })
        .register_command(Command {
            id: "refresh",
            summary: "Refresh server list",
            syntax: None,
            category: Some("servers"),
            execute: refresh_servers,
        });
    
    // Build and run
    let mut app = builder.build(ctx)?;
    
    // Initial data refresh
    // In a real app, you'd get the view and refresh it here
    
    app.run()?;
    
    Ok(())
}
```

## Step 8: Run Your Application

```bash
cargo run
```

You should see:
- A header showing server count and status
- A grid view displaying your servers
- Status bar at the bottom
- Press `?` for help overlay
- Press `:` to open command palette
- Press `1` to switch to servers view
- Press `q` to quit

## Next Steps

### Add Multiple Views

```rust
struct LogsView;

impl View for LogsView {
    fn id(&self) -> &'static str { "logs" }
    fn title(&self) -> &'static str { "Logs" }
    // ... implement other methods
}

// In main():
builder = builder
    .register_view(ServersView::new())
    .register_view(LogsView)
    .map_view_slot(ViewSlot::Slot1, "servers")
    .map_view_slot(ViewSlot::Slot2, "logs");
```

### Add LogView with Streaming

```rust
use tui_framework::widget::LogView;
use tui_framework::data_source::{SharedLogBuffer, sync_log_buffer_to_view};

struct LogsView {
    log_view: LogView,
    log_buffer: SharedLogBuffer,
}

impl LogsView {
    fn new() -> Self {
        let theme = Theme::default();
        let log_view = LogView::new(theme);
        let log_buffer = SharedLogBuffer::new(10000);
        
        // Start background thread to add logs
        let buffer_clone = log_buffer.clone();
        std::thread::spawn(move || {
            let mut counter = 0;
            loop {
                std::thread::sleep(std::time::Duration::from_millis(1000));
                counter += 1;
                buffer_clone.push(format!("Log message #{}", counter));
            }
        });
        
        Self { log_view, log_buffer }
    }
}

impl View for LogsView {
    // ... implement View trait
    fn render(&mut self, f: &mut Frame, area: Rect, _ctx: &dyn AppContext) {
        sync_log_buffer_to_view(&self.log_buffer, &mut self.log_view);
        self.log_view.render(f, area);
    }
}
```

### Customize Keybindings

```rust
use tui_framework::keymap::{KeymapConfig, KeyBinding, AppCommand};

let mut keymap = KeymapConfig::new();
keymap = keymap.add_global(KeyBinding::new(
    KeyCode::Char('r'),
    AppCommand::InvokeAction("refresh".to_string()),
));

builder = builder.configure_keymap(keymap);
```

### Add Error Handling

```rust
fn handle_event(&mut self, event: &Event, ctx: &mut dyn AppContext) -> ViewResult {
    if let Event::Key(key) = event {
        match key.code {
            KeyCode::Char('r') => {
                match self.grid.data_source_mut().refresh(ctx) {
                    Ok(_) => ViewResult::Handled,
                    Err(e) => ViewResult::ShowModal(
                        AppMessage::error(
                            "Refresh failed".to_string(),
                            Some(e.to_string()),
                        ),
                    ),
                }
            }
            // ... other handlers
        }
    }
    ViewResult::Ignored
}
```

## Common Patterns

### Connecting to Real Services

```rust
struct MyServiceClient {
    base_url: String,
}

impl MyServiceClient {
    fn list_servers(&self) -> Result<Vec<Server>> {
        // Make HTTP request to your API
        // Parse response into Server structs
        Ok(vec![])
    }
}

struct MyAppContext {
    client: MyServiceClient,
}

impl AppContext for MyAppContext {}

// In DataSource::refresh:
fn refresh(&mut self, ctx: &dyn AppContext) -> Result<()> {
    // Cast to your context type (in real app, use proper trait methods)
    // self.servers = ctx.client.list_servers()?;
    Ok(())
}
```

### Paginated DataSource

```rust
use std::collections::HashMap;

struct PaginatedDataSource {
    page_cache: HashMap<usize, Vec<Server>>,
    page_size: usize,
    total: usize,
    client: MyServiceClient,
}

impl DataSource for PaginatedDataSource {
    type Row = Server;

    fn len(&self) -> usize {
        self.total
    }

    fn get(&self, index: usize) -> Option<&Server> {
        let page = index / self.page_size;
        let offset = index % self.page_size;
        
        // Load page if not in cache (triggered by framework)
        self.page_cache.get(&page)?.get(offset)
    }

    fn refresh(&mut self, _ctx: &dyn AppContext) -> Result<()> {
        // Fetch first page or all data
        // Update total count
        Ok(())
    }
}
```

## Troubleshooting

### View not showing
- Check that view is registered before `build()`
- Verify view ID matches in `map_view_slot()`
- Ensure at least one view is registered

### Command not found
- Verify command is registered before `build()`
- Check command ID matches what you're typing
- Ensure command palette is enabled (default: enabled)

### Data not refreshing
- Call `refresh()` explicitly in event handlers
- Check error messages in status bar
- Verify `AppContext` has valid clients

### Keybinding not working
- Check keybinding priority (view-specific > global)
- Verify keybinding is registered for correct view
- Check for conflicts (framework will warn)

## Resources

- [Examples Guide](../EXAMPLES.md) - See working examples
- [Full Specification](../specs/001-cli-framework-spec/spec.md) - Complete specification
- [API Documentation](https://docs.rs/tui-framework) - Generated API docs (when published)

