# TUI Framework

An opinionated TUI framework library for building terminal user interfaces in Rust.

This framework provides a complete event loop, layout system, navigation, status bar, help overlay, command palette, and standard widgets (GridView, LogView, ModalView) so application authors can focus on implementing views, datasources, and commands rather than terminal management.

## Features

- **View System**: Register views, switch between them with F-keys (F1-F12)
- **Command Palette**: Execute commands via `:` key with filtering
- **Keybindings**: Global and per-view keybindings with conflict resolution
- **GridView**: Paginated data display with DataSource trait
- **LogView**: Streaming logs with filtering and follow mode
- **UI Components**: Status bar, help overlay, modal dialogs
- **Customization**: Toggle UI elements, configure keybindings
- **Resilience**: Retry policies for network operations
- **Optional Features**: Authentication hooks, OpenTelemetry integration

## Quick Start

Add to your `Cargo.toml`:

```toml
[dependencies]
tui-framework = { path = "../tui-framework" }  # or from crates.io when published
anyhow = "1.0"
```

Basic example:

```rust
use tui_framework::prelude::*;
use tui_framework::view::{View, ViewResult, HelpItem};
use crossterm::event::Event;
use ratatui::layout::Rect;
use ratatui::Frame;

// Define a simple view
struct MyView;
impl View for MyView {
    fn id(&self) -> &'static str { "my.view" }
    fn title(&self) -> &'static str { "My View" }
    fn render(&mut self, _f: &mut Frame, _area: Rect, _ctx: &dyn AppContext) {}
    fn handle_event(&mut self, _event: &Event, _ctx: &mut dyn AppContext) -> ViewResult {
        ViewResult::Ignored
    }
    fn help_items(&self) -> Vec<HelpItem> { vec![] }
}

// Build and run
struct MyContext;
impl AppContext for MyContext {}

fn main() -> anyhow::Result<()> {
    let mut builder = AppBuilder::new();
    builder = builder
        .register_view(MyView)
        .map_view_slot(ViewSlot::F1, "my.view");
    let mut app = builder.build(MyContext)?;
    app.run()?;
    Ok(())
}
```

## Running Examples

The framework includes several examples demonstrating different features:

### Simple Example
Basic TUI with a single grid view:
```bash
cargo run --example simple
```

### With Commands Example
Demonstrates command palette, keybindings, and multiple views:
```bash
cargo run --example with_commands
```

### Kitchen Sink Example
Comprehensive example showing all framework features:
```bash
cargo run --example kitchen_sink
```

## Key Bindings

Default keybindings:
- `F1-F12`: Switch to mapped views
- `:`: Open command palette
- `?`: Toggle help overlay
- `q`: Quit application
- `Esc`: Close modals/overlays

## Documentation

- [Quickstart Guide](specs/001-cli-framework-spec/quickstart.md) - Detailed getting started guide
- [Specification](specs/001-cli-framework-spec/spec.md) - Full specification
- [API Documentation](https://docs.rs/tui-framework) - Generated API docs (when published)

## Requirements

- Rust 1.70+ (2021 edition)
- Terminal with at least 80x24 characters (gracefully degrades for smaller terminals)

## License

MIT OR Apache-2.0

## Contributing

See [CONTRIBUTING.md](CONTRIBUTING.md) for guidelines on contributing to this project.

