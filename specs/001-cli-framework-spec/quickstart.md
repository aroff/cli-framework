# Quickstart Guide: CLI Framework

**Purpose**: Get a basic TUI console up and running in under 30 minutes.

## Prerequisites

- Rust toolchain (latest stable)
- Basic familiarity with Rust
- Terminal with at least 80x24 characters

## Step 1: Add Dependency

Add to your `Cargo.toml`:

```toml
[dependencies]
tui-framework = { path = "../tui-framework" }  # or from crates.io when published
anyhow = "1.0"
```

## Step 2: Define Your Data Model

```rust
// src/main.rs
use tui_framework::prelude::*;

// Your application's data model
#[derive(Clone)]
struct Server {
    id: String,
    name: String,
    status: String,
}

// Your service client (simplified example)
struct MyServiceClient {
    // Your HTTP/gRPC client here
}

impl MyServiceClient {
    fn list_servers(&self) -> anyhow::Result<Vec<Server>> {
        // Your actual API call
        Ok(vec![
            Server { id: "1".to_string(), name: "api-1".to_string(), status: "running".to_string() },
            Server { id: "2".to_string(), name: "api-2".to_string(), status: "stopped".to_string() },
        ])
    }
}
```

## Step 3: Implement DataSource

```rust
use tui_framework::data_source::DataSource;

struct ServersDataSource {
    servers: Vec<Server>,
    client: MyServiceClient,
}

impl DataSource for ServersDataSource {
    type Row = Server;

    fn len(&self) -> usize {
        self.servers.len()
    }

    fn get(&self, index: usize) -> Option<&Server> {
        self.servers.get(index)
    }

    fn refresh(&mut self, ctx: &AppContext) -> anyhow::Result<()> {
        self.servers = ctx.clients.my_service.list_servers()?;
        Ok(())
    }
}
```

## Step 4: Create a View

```rust
use tui_framework::view::View;
use tui_framework::widget::GridView;
use ratatui::prelude::*;

struct ServersView {
    grid: GridView<ServersDataSource>,
}

impl ServersView {
    fn new(client: MyServiceClient) -> Self {
        let data_source = ServersDataSource {
            servers: Vec::new(),
            client,
        };
        let columns = vec![
            ColumnSpec::new("ID", |s: &Server| s.id.clone(), 10),
            ColumnSpec::new("Name", |s: &Server| s.name.clone(), 20),
            ColumnSpec::new("Status", |s: &Server| s.status.clone(), 15),
        ];
        Self {
            grid: GridView::new(data_source, columns),
        }
    }
}

impl View for ServersView {
    fn id(&self) -> &'static str {
        "servers"
    }

    fn title(&self) -> &'static str {
        "Servers"
    }

    fn render(&mut self, f: &mut Frame, area: Rect, ctx: &AppContext) {
        self.grid.render(f, area, ctx);
    }

    fn handle_event(&mut self, event: &Event, ctx: &mut AppContext) -> ViewResult {
        self.grid.handle_event(event, ctx)
    }

    fn help_items(&self) -> Vec<HelpItem> {
        vec![
            HelpItem::new("r", "Refresh servers"),
            HelpItem::new("Enter", "View server details"),
        ]
    }
}
```

## Step 5: Define AppContext

```rust
use tui_framework::app::AppContext;

// Your application's context
struct MyAppContext {
    clients: MyClients,
    // Add other application state here
}

struct MyClients {
    my_service: MyServiceClient,
}

// Framework will use this via trait bounds
impl AppContext for MyAppContext {
    // Framework provides helpers, you define structure
}
```

## Step 6: Build and Run

```rust
use tui_framework::app::AppBuilder;
use tui_framework::view::ViewSlot;

fn main() -> anyhow::Result<()> {
    // Initialize your service clients
    let service_client = MyServiceClient::new();
    
    // Create your app context
    let app_context = MyAppContext {
        clients: MyClients {
            my_service: service_client,
        },
    };

    // Build the app
    let app = AppBuilder::new()
        .register_view(ServersView::new(app_context.clients.my_service.clone()))
        .map_view_slot(ViewSlot::F1, "servers")
        .build(app_context)?;

    // Run the TUI
    app.run()?;

    Ok(())
}
```

## Step 7: Run Your Application

```bash
cargo run
```

You should see:
- Your servers list in a grid
- Status bar at the bottom
- Press `?` for help
- Press `F1` to switch to servers view (if you have multiple views)
- Press `q` to quit

## Next Steps

### Add a Command

```rust
use tui_framework::command::{Command, CommandArgs};

fn restart_server(ctx: &mut AppContext, args: CommandArgs) -> anyhow::Result<()> {
    let server_id = args.named.get("server")
        .ok_or_else(|| anyhow::anyhow!("Missing 'server' argument"))?;
    
    ctx.clients.my_service.restart_server(server_id)?;
    
    ctx.push_message(AppMessage::info(
        "Server restarted".to_string(),
        Some(format!("Server {} restarted successfully", server_id)),
    ));
    
    Ok(())
}

// In main():
let app = AppBuilder::new()
    .register_view(ServersView::new(...))
    .register_command(Command {
        id: "restart",
        summary: "Restart a server",
        syntax: Some(":restart server=<id>"),
        execute: restart_server,
    })
    .build(app_context)?;
```

### Add Multiple Views

```rust
let app = AppBuilder::new()
    .register_view(ServersView::new(...))
    .register_view(NetworksView::new(...))
    .register_view(LogsView::new(...))
    .map_view_slot(ViewSlot::F1, "servers")
    .map_view_slot(ViewSlot::F2, "networks")
    .map_view_slot(ViewSlot::F3, "logs")
    .build(app_context)?;
```

### Customize Keybindings

```rust
use tui_framework::keymap::{KeymapConfig, KeyBinding, AppCommand};

let keymap = KeymapConfig {
    global: vec![
        KeyBinding::new(Key::Char('r'), AppCommand::InvokeAction("refresh")),
    ],
    per_view: vec![
        ("servers".into(), vec![
            KeyBinding::new(Key::Char('t'), AppCommand::RunCommand("restart", CommandArgs::default())),
        ]),
    ],
};

let app = AppBuilder::new()
    .register_view(ServersView::new(...))
    .configure_keymap(keymap)
    .build(app_context)?;
```

### Enable Observability

```rust
use tui_framework::observability::ObservabilityConfig;

let app = AppBuilder::new()
    .register_view(ServersView::new(...))
    .with_observability(ObservabilityConfig::opentelemetry()?)
    .build(app_context)?;
```

## Common Patterns

### Handling Errors in DataSource

```rust
fn refresh(&mut self, ctx: &AppContext) -> anyhow::Result<()> {
    match ctx.clients.my_service.list_servers() {
        Ok(servers) => {
            self.servers = servers;
            Ok(())
        }
        Err(e) => {
            // Framework will show error via AppMessage
            Err(anyhow::anyhow!("Failed to fetch servers: {}", e))
        }
    }
}
```

### Custom View with State

```rust
struct MyView {
    selected_index: usize,
    filter: String,
    // ... other state
}

impl View for MyView {
    // State persists when switching views
    // Framework calls render() and handle_event() on same instance
}
```

### Paginated DataSource

```rust
struct PaginatedDataSource {
    page_cache: HashMap<usize, Vec<MyRow>>,
    page_size: usize,
    total: usize,
}

impl DataSource for PaginatedDataSource {
    fn get(&self, index: usize) -> Option<&MyRow> {
        let page = index / self.page_size;
        let offset = index % self.page_size;
        
        // Load page if not in cache
        if !self.page_cache.contains_key(&page) {
            // Trigger page load (framework handles this)
        }
        
        self.page_cache.get(&page)?.get(offset)
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
- Call `refresh()` explicitly or configure auto-refresh
- Check error messages in status bar
- Verify `AppContext` has valid clients

### Keybinding not working
- Check keybinding priority (view-specific > global)
- Verify keybinding is registered for correct view
- Check for conflicts (framework will warn)

## Resources

- [Full API Documentation](./contracts/)
- [Data Model Reference](./data-model.md)
- [Architecture Overview](./plan.md)

