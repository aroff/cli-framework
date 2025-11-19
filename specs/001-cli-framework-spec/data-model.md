# Data Model: CLI Framework – Opinionated TUI Library

**Date**: 2025-11-17  
**Feature**: 001-cli-framework-spec

## Core Entities

### View

**Purpose**: Represents a screen within the TUI (e.g., list of jobs, details of a resource, log stream).

**Attributes**:
- `id: &'static str` - Stable identifier (compile-time literal)
- `title: &'static str` - Human-readable name shown in status bar/tabs

**Behavior**:
- `render(&mut self, f: &mut Frame, area: Rect, ctx: &AppContext)` - Draws the view
- `handle_event(&mut self, event: &Event, ctx: &mut AppContext) -> ViewResult` - Handles keyboard events
- `help_items(&self) -> Vec<HelpItem>` - Returns view-specific help items

**State Management**:
- Views are created once at startup and persist state (scroll position, selection, filters) when switching between views
- Framework manages view lifecycle; applications implement the trait

**Relationships**:
- Registered with `AppBuilder` via `register_view()`
- Mapped to view slots (F1-F12) via `map_view_slot()`
- Can have per-view keybindings via `KeymapConfig`

**Validation Rules**:
- View IDs must be unique within an application
- Views must handle all events gracefully (return appropriate `ViewResult`)

### DataSource

**Purpose**: Provider of tabular data for `GridView`, abstracting data fetching and pagination.

**Attributes**:
- `Row` (associated type) - The type of data row

**Behavior**:
- `len(&self) -> usize` - Total number of rows (logical length)
- `get(&self, index: usize) -> Option<&Self::Row>` - Access a row by index (may trigger pagination)
- `refresh(&mut self, ctx: &AppContext) -> anyhow::Result<()>` - Refresh underlying data

**State Management**:
- Applications manage internal state (cache, pagination info, etc.)
- Framework calls `refresh()` when data needs updating
- Framework calls `get()` for visible rows during rendering

**Relationships**:
- Used by `GridView<D: DataSource>` widget
- Applications implement this trait for their data sources

**Validation Rules**:
- `get(index)` must return `None` if `index >= len()`
- `refresh()` should handle errors gracefully and return `Result`

**Implementation Patterns**:
- **In-memory**: Store `Vec<Row>`, implement `len()` and `get()` over the vector
- **Paginated**: Keep page cache, load pages on-demand when `get()` is called

### Command

**Purpose**: Represents an executable operation (e.g., restarting a service, triggering a job).

**Attributes**:
- `id: CommandId` (`&'static str`) - Unique command identifier
- `summary: &'static str` - Short description (shown in command palette)
- `syntax: Option<&'static str>` - Optional syntax hint (e.g., `":restart service=<name> env=<env>"`)
- `category: Option<&'static str>` - Optional category for grouping in palette
- `execute: fn(&mut AppContext, CommandArgs) -> CommandResult` - Execution function

**Behavior**:
- Executed via command palette (`:command arg=value`) or keybindings
- Receives parsed `CommandArgs` (positional and named arguments)
- Returns `CommandResult` (wrapped `anyhow::Result<()>`)
- Can push `AppMessage` for user feedback

**State Management**:
- Commands are stateless (pure functions)
- State changes happen via `AppContext` mutations

**Relationships**:
- Registered with `AppBuilder` via `register_command()`
- Can be invoked via command palette or keybindings
- Uses `AppContext` for service access

**Validation Rules**:
- Command IDs must be unique within an application
- Execution functions should validate arguments and return appropriate errors
- Commands should provide user feedback via `AppMessage`

### AppContext

**Purpose**: Application-owned state and service clients that views, datasources, and commands use.

**Attributes**:
- Defined by the application (framework doesn't impose structure)
- Typical fields: service clients (REST/gRPC), configuration, metrics sources

**Behavior**:
- Passed to views for rendering (immutable reference)
- Passed to event handlers and refresh operations (mutable reference)
- Applications manage lifecycle and initialization

**State Management**:
- Created by application before building `App`
- Owned by application, framework only borrows references
- Can contain clients, caches, configuration, etc.

**Relationships**:
- Used by all framework operations (View::render, DataSource::refresh, Command::execute)
- Applications define structure based on their needs

**Validation Rules**:
- Must be initialized before `AppBuilder::build()`
- Should be `Send + Sync` if applications want to use with async runtimes (v2+)

### AppMessage

**Purpose**: User-visible messages with different severity levels and detail levels.

**Attributes**:
- `kind: AppMessageKind` - Info, Warning, or Error
- `short: String` - One-line text for status bar
- `details: Option<String>` - Optional detailed text for modal

**Behavior**:
- Created by framework operations or applications
- Short message shown in status bar
- Detailed message shown in modal (if provided) when user requests

**State Management**:
- Framework manages message queue/display
- Latest message shown in status bar
- Detailed messages stored until dismissed

**Relationships**:
- Generated by framework operations (retry failures, command execution)
- Generated by applications (custom operations, validation errors)

**Validation Rules**:
- Short message should be concise (fits in status bar)
- Details should provide actionable information

### Theme

**Purpose**: Centralizes styling definitions (colors, modifiers) to ensure consistent UI across views and standard widgets.

**Attributes**:
- `primary_style: Style` - For focused/selected items
- `secondary_style: Style` - For normal text
- `error_style: Style` - For error messages/states
- `status_bar_style: Style` - For the bottom status bar
- `modal_style: Style` - For modal borders/backgrounds

**Behavior**:
- Defaults provided by framework (opinionated default theme)
- Can be overridden by application at startup
- Passed to views/widgets implicitly or via context

**Relationships**:
- Used by `GridView`, `LogView`, `StatusBar`, `ModalView` to derive colors

### Module

**Purpose**: A grouping of related views, commands, and keybindings to support internal modularization (FR-009).

**Behavior**:
- `id() -> &'static str` - Returns a stable identifier for the module
- `register(builder: &mut AppBuilder) -> anyhow::Result<()>` - Registers its components with the builder, returns error on registration conflicts

**Relationships**:
- Implemented by application code to organize features (e.g. `AirflowModule`, `HetznerModule`)
- Called during app initialization

### KeyBinding

**Purpose**: Maps keyboard input to framework actions (view switching, command execution, etc.).

**Attributes**:
- `key: Key` - The keyboard key or key sequence
- `action: AppCommand` - The action to execute (SwitchView, InvokeAction, RunCommand)

**Behavior**:
- Resolved by framework during event processing
- Priority: modals > view-specific > global

**State Management**:
- Configured via `KeymapConfig` at application startup
- Framework maintains keymap registry

**Relationships**:
- Can be global (applies to all views) or per-view
- Maps to `AppCommand` enum variants

**Validation Rules**:
- Keybindings must not conflict in same priority level (framework reports conflicts)
- View-specific bindings override global bindings

### HelpItem

**Purpose**: Represents a help entry (keybinding and description) shown in help overlay.

**Attributes**:
- `key: String` - The key or key sequence (e.g., "F1", "t", "Ctrl+C")
- `description: String` - What the keybinding does

**Behavior**:
- Shown in help overlay (triggered by `?`)
- Merged from global help and view-specific help

**State Management**:
- Generated dynamically from keymap and view help items
- Framework manages display

**Relationships**:
- Global help items from framework defaults
- View-specific help items from `View::help_items()`

## State Transitions

### View Lifecycle

```
[Created at startup] → [Active] ↔ [Inactive] → [Destroyed on app exit]
                         ↓
                    [State persisted]
```

- Views created once during `AppBuilder::build()`
- Switching views: Active → Inactive (state preserved) → Active (state restored)
- Views destroyed when `App` is dropped

### Command Execution Flow

```
[User input] → [Parse command] → [Validate args] → [Execute] → [Show result]
                                      ↓ (error)
                                 [Show error message]
```

- Command parsing happens in framework
- Argument validation in command implementation
- Execution uses `AppContext` for service access
- Results shown via `AppMessage`

### Runtime Loop

```
[Input Event] → [Keymap Resolution] → [Active View Handle] → [Global Handle] → [State Update] → [Render]
```

1.  **Input Event**: `crossterm` event received (Key, Resize, Mouse).
2.  **Keymap Resolution**:
    *   Check **Modal** bindings (if modal active).
    *   Check **View-specific** bindings (for active view).
    *   Check **Global** bindings.
    *   If match found → convert to `AppCommand`.
3.  **Active View Handle**: If no binding matched, pass raw event to `active_view.handle_event()`.
4.  **Global Handle**: If view ignored event, framework handles global defaults (e.g. `q` to quit if not captured).
5.  **State Update**: Execute `AppCommand` (switch view, run command) or apply `ViewResult` (show modal, exit).
6.  **Render**:
    *   Clear buffer.
    *   Draw **Active View**.
    *   Draw **Status Bar** (overlay).
    *   Draw **Modal** (if active, overlay).
    *   Draw **Help** (if active, overlay).
    *   Flush to terminal.

### Data Refresh Flow

```
[Trigger refresh] → [Apply retry policy] → [Call DataSource::refresh] → [Update UI]
                         ↓ (failure)
                    [Retry or show error]
```

- Framework manages retry logic
- Applications implement actual data fetching
- UI updates automatically after successful refresh

## Data Volume Assumptions

- **Grid rows**: Optimize for thousands of rows (no hard limit)
- **Log lines**: Optimize for tens of thousands of lines (no hard limit)
- **Views**: Typically 1-12 views per application (F1-F12 slots)
- **Commands**: Typically 5-50 commands per application
- **Keybindings**: Typically 10-30 keybindings per application

## Validation and Constraints

### View IDs
- Must be `&'static str` (compile-time literals)
- Must be unique within an application
- Framework validates uniqueness at registration

### Command IDs
- Must be `&'static str` (compile-time literals)
- Must be unique within an application
- Framework validates uniqueness at registration

### Terminal Size
- Minimum: 80x24 characters
- Framework gracefully degrades for smaller terminals
- Applications don't need to handle terminal resizing (framework does)

### Keybinding Conflicts
- Framework reports conflicts at startup
- Resolution: view-specific > global, modals > everything
- Applications can override defaults via `KeymapConfig`

