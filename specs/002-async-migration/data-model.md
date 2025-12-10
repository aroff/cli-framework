# Data Model: Async Runtime Migration

**Date**: 2025-12-09  
**Feature**: 002-async-migration

## Core Entities

### AsyncRuntime

**Purpose**: The underlying async runtime (Tokio) that manages async tasks, event loop, and concurrent operations.

**Attributes**:
- Runtime handle: `tokio::runtime::Runtime` (internal, not exposed)
- Task manager: Background task tracking and cancellation
- Event loop: Async event processing

**Behavior**:
- Created automatically by framework (FR-018)
- Manages all async operations
- Handles task spawning, cancellation, and cleanup
- Provides async context for all framework operations

**State Management**:
- Initialized once during `AppBuilder::build()`
- Lives for the lifetime of the `App`
- Cleaned up when `App` is dropped

**Relationships**:
- Owned by `App` struct
- Used by all async operations (DataSource, View, Command)
- Manages background tasks

**Validation Rules**:
- Must be initialized successfully or framework fails to start
- Must handle runtime errors gracefully (show error messages)

### AsyncDataSource

**Purpose**: A DataSource implementation that performs async I/O operations (network, database) during refresh.

**Attributes**:
- `Row` (associated type) - The type of data row
- Internal cache/state (implementation-specific)

**Behavior**:
- `len(&self) -> usize` - Total number of rows (sync, fast)
- `get(&self, index: usize) -> Option<&Self::Row>` - Access row (sync, may trigger async fetch)
- `async fn refresh(&mut self, ctx: &dyn AppContext) -> Result<()>` - Async refresh operation

**State Management**:
- Applications manage internal state (cache, pagination, etc.)
- Framework calls `refresh()` when data needs updating
- Framework calls `get()` for visible rows during rendering

**Relationships**:
- Used by `GridView<D: DataSource>` widget
- Applications implement this trait for their data sources
- Uses `AppContext` for service access

**Validation Rules**:
- `get(index)` must return `None` if `index >= len()`
- `refresh()` must handle errors gracefully
- Must be `Send + Sync` for thread safety

**Implementation Patterns**:
- **In-memory**: Store `Vec<Row>`, implement `len()` and `get()` over the vector
- **Paginated**: Keep page cache, load pages on-demand when `get()` is called
- **Network-backed**: Fetch data in `refresh()`, cache results

### AsyncCommand

**Purpose**: A Command that executes async operations and can use `.await` for service calls.

**Attributes**:
- `id: CommandId` (`&'static str`) - Unique command identifier
- `summary: &'static str` - Short description
- `syntax: Option<&'static str>` - Optional syntax hint
- `category: Option<&'static str>` - Optional category
- `execute: fn(&mut dyn AppContext, CommandArgs) -> Pin<Box<dyn Future<Output = CommandResult> + Send>>` - Async execution function

**Behavior**:
- Executed via command palette or keybindings
- Receives parsed `CommandArgs` (positional and named arguments)
- Returns `CommandResult` (wrapped `anyhow::Result<()>`)
- Can push `AppMessage` for user feedback
- Can use `.await` for async operations

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

### BackgroundTask

**Purpose**: A long-running async operation that updates the UI when complete, without blocking the event loop.

**Attributes**:
- Task handle: `tokio::task::JoinHandle` (internal)
- Cancellation token: `CancellationToken` (for cancellation)
- Result channel: `mpsc::Receiver<TaskResult>` (for receiving results)

**Behavior**:
- Spawned by framework or applications
- Runs concurrently with main event loop
- Sends results back via channel when complete
- Can be cancelled via cancellation token

**State Management**:
- Created when operation is spawned
- Tracked by framework's background task manager
- Cleaned up when task completes or is cancelled

**Relationships**:
- Managed by `BackgroundTaskManager` (new component)
- Results consumed by main event loop
- Used for streaming data, long operations

**Validation Rules**:
- Must handle cancellation gracefully
- Must send results via channel (non-blocking)
- Must not block the async runtime

### AsyncEventLoop

**Purpose**: The main event loop that handles terminal events, async operations, and rendering in a non-blocking manner.

**Attributes**:
- Terminal: `Terminal<CrosstermBackend<io::Stdout>>`
- Event reader: Async terminal event reading
- Background task receiver: `mpsc::Receiver<TaskResult>`
- Render interval: `tokio::time::Interval` (for periodic rendering)

**Behavior**:
- Reads terminal events asynchronously
- Processes async operations (DataSource refresh, Command execution)
- Renders UI (synchronously, from async context)
- Handles background task results
- Manages cancellation and timeouts

**State Management**:
- Runs continuously until application exits
- Maintains view state, command state, etc.
- Tracks active async operations for loading indicators

**Relationships**:
- Part of `App::run()` method
- Uses `Runtime` for terminal management
- Coordinates with `ViewRegistry`, `CommandRegistry`, `KeymapResolver`

**Validation Rules**:
- Must maintain ≤16ms frame time (SC-007)
- Must respond to user input within 50ms during async operations (SC-002)
- Must handle errors gracefully without crashing

### AppContext (Updated)

**Purpose**: Application-owned state and service clients that views, datasources, and commands use.

**Attributes**:
- Defined by the application (framework doesn't impose structure)
- Typical fields: service clients (REST/gRPC), configuration, metrics sources

**Behavior**:
- Passed to views for rendering (immutable reference)
- Passed to event handlers and refresh operations (mutable reference)
- Must be `Send + Sync` for thread safety (FR-007)

**State Management**:
- Applications own and manage their context
- Framework only requires the trait to exist
- Context lives for the lifetime of the `App`

**Relationships**:
- Used by all views, datasources, and commands
- Applications implement this trait

**Validation Rules**:
- MUST be `Send + Sync` (compile-time enforced)
- Applications using non-Send types (e.g., `Rc`) must use `Arc` instead
- Applications with non-Sync data need synchronization primitives

## State Transitions

### Async Operation Lifecycle

```
[Created] → [Running] → [Completed] → [UI Updated]
              ↓
         [Cancelled] → [Cleaned Up]
              ↓
         [Timeout] → [Error Shown] → [Cleaned Up]
```

- Operations created when triggered (refresh, command, etc.)
- Running state tracked for loading indicators
- Completed operations update UI
- Cancelled operations are cleaned up gracefully
- Timeout operations show error and clean up

### Background Task Lifecycle

```
[Spawned] → [Running] → [Result Sent] → [Consumed] → [Cleaned Up]
              ↓
         [Cancelled] → [Cleaned Up]
```

- Tasks spawned for long-running operations
- Results sent via channel (non-blocking)
- Main loop consumes results and updates UI
- Tasks cleaned up when complete or cancelled

### View State During Async Operations

```
[Active] → [Async Operation Started] → [Loading Indicator Shown] → [Operation Complete] → [UI Updated]
              ↓ (view switch)
         [Operation Cancelled] → [New View Active]
```

- Views remain active during async operations
- Loading indicators shown automatically
- Operations cancelled if view switches
- UI updates when operations complete

## Data Flow

### DataSource Refresh Flow

```
[User Action] → [Framework Calls refresh()] → [Async Operation Starts] → [Loading Indicator Shown]
                                                                              ↓
[Network/DB I/O] → [Data Fetched] → [Cache Updated] → [Loading Indicator Hidden] → [UI Rendered with New Data]
```

### Command Execution Flow

```
[User Input] → [Command Parsed] → [Async execute() Called] → [Loading Indicator Shown]
                                                                  ↓
[Service Call] → [Operation Complete] → [Result Processed] → [Loading Indicator Hidden] → [Status Message Shown]
```

### Background Task Flow

```
[Task Spawned] → [Runs Concurrently] → [Results Sent via Channel] → [Main Loop Receives] → [UI Updated]
```

## Validation Rules Summary

1. **AppContext**: MUST be `Send + Sync` (compile-time)
2. **DataSource**: `get(index)` returns `None` if `index >= len()`
3. **Async Operations**: Must handle cancellation and timeout gracefully
4. **Background Tasks**: Must not block the async runtime
5. **Event Loop**: Must maintain performance targets (16ms frame, 50ms interaction)

