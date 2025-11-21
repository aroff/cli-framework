# TUI Framework

An opinionated TUI framework library for building terminal user interfaces in Rust.

This framework provides a complete event loop, layout system, navigation, status bar, help overlay, command palette, and standard widgets (GridView, LogView, ModalView) so application authors can focus on implementing views, datasources, and commands rather than terminal management.

## Features

### Core Framework

- **View System**: Register views, switch between them with numeric keys (1-9)
- **View Headers**: Optional contextual information and keybindings displayed in view headers with dynamic height
- **Event Loop**: Single-threaded synchronous event loop with terminal I/O management
- **AppBuilder**: Builder pattern for configuring views, commands, keybindings, and UI toggles
- **AppContext**: Trait for application-owned state and service clients

### Views & Navigation

- **View Trait**: Implement views with rendering, event handling, and help items
- **View Registry**: Manage multiple views with persistent state
- **View Switching**: Map views to numeric keys (1-9) for quick navigation
- **View Headers**: Display contextual information (left), centered title, and keybindings (right)

### Commands & Actions

- **Command System**: Formal command abstraction with ID, summary, syntax, and category
- **Command Palette**: Execute commands via `:` key with filtering and autocomplete
- **Command Registry**: Register and manage application commands
- **Command Parser**: Parse command syntax with positional and named arguments

### Data Display

- **GridView**: Paginated data display widget with DataSource trait integration
- **DataSource Trait**: Uniform API for in-memory lists and lazy page loading
- **LogView**: Streaming logs with scrolling, follow mode, and keyword filtering
- **Empty States**: Standard empty state messages and loading indicators

### UI Components

- **Status Bar**: Bottom status bar for messages and application state
- **Help Overlay**: Context-sensitive help display (press `?`)
- **Modal Dialogs**: Standard modal dialogs for messages and confirmations
- **View Headers**: Optional headers with contextual info and keybindings

### Keybindings

- **Global Keybindings**: Application-wide keybindings
- **Per-View Keybindings**: View-specific keybindings that override global
- **Keymap Configuration**: Configure keybindings programmatically
- **Conflict Resolution**: Priority system (modals > view-specific > global)

### Customization

- **UI Toggles**: Enable/disable status bar, help overlay, command palette
- **Theme System**: Centralized styling with standard ANSI 16-color palette
- **Keymap Customization**: Configure global and per-view keybindings
- **Module System**: Optional internal modularization for grouping components

### Resilience & Error Handling

- **Retry Policies**: Configurable retry strategies for network operations
- **Timeout Handling**: Configurable timeout handling for operations
- **Error Messages**: Standard AppMessage model for user-visible errors
- **Graceful Degradation**: Handles small terminals (minimum 80x24) gracefully

### Optional Features

- **Authentication**: Optional built-in authentication mechanisms (login screen, token management, RBAC)
- **Observability**: Optional OpenTelemetry integration for logging, metrics, and tracing
- **Module Trait**: Optional internal modularization for application organization

### Terminal Support

- **Minimum Size**: 80x24 characters with graceful degradation
- **Size Handling**: Scrollable content, truncated labels, minimum functional area preserved
- **ANSI Colors**: Standard 16-color palette for compatibility

## Quick Start

Add to your `Cargo.toml`:

```toml
[dependencies]
tui-framework = { path = "../tui-framework" }  # or from crates.io when published
anyhow = "1.0"
```

See [Getting Started Tutorial](docs/getting-started.md) for a complete walkthrough.

## Examples

The framework includes several examples demonstrating different features. See [EXAMPLES.md](EXAMPLES.md) for details:

- `cargo run --example simple` - Basic TUI with single view
- `cargo run --example with_commands` - Command palette and multiple views
- `cargo run --example kitchen_sink` - Comprehensive feature demonstration

## Key Bindings

Default keybindings:
- `1-9`: Switch to mapped views
- `:`: Open command palette
- `?`: Toggle help overlay
- `q`: Quit application
- `Esc`: Close modals/overlays

## Documentation

- [Getting Started Tutorial](docs/getting-started.md) - Step-by-step tutorial for creating your first project
- [Examples Guide](EXAMPLES.md) - Detailed examples and usage patterns
- [Specification](specs/001-cli-framework-spec/spec.md) - Full specification
- [API Documentation](https://docs.rs/tui-framework) - Generated API docs (when published)

## Requirements

- Rust 1.70+ (2021 edition)
- Terminal with at least 80x24 characters (gracefully degrades for smaller terminals)

## License

Apache-2.0

## Contributing

See [CONTRIBUTING.md](CONTRIBUTING.md) for guidelines on contributing to this project.

