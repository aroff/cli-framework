## 0. Goals and scope

We want an **opinionated TUI framework library** for a **single CLI application** (a service “XYZ” per binary).  
The framework should:

- **Minimize hosting code** for the CLI developer:
  - The app author focuses on **commands and operations**.
  - The framework takes care of **event loop, layout, navigation, status bar, help, command palette, keymap**, etc.
- Be used via **static linking**, not through dynamically loaded plugins.
- Optionally allow the same binary to also **serve a REST or gRPC API** for the service.
- Provide a path to **“plugin-like” modularization inside the app** if desired (internal modules), but not a multi-service / multi-tenant “plugin host”.

The primary use case: the CLI is a **monitoring and operations console** for a single service, occasionally issuing actions (restart, scale, trigger, etc.) and often just showing state.

---

## 1. Conceptual organization

Imagine a crate `tui-framework` with three main pieces:

1. **Core app runtime**

   - Event loop (e.g., `crossterm` + `ratatui`).
   - Manages views, routing, global keybindings.
   - Draws status bar, help overlay, command palette, standard modals.
   - Provides a standard API for **messages** (short in status bar, detailed in modal).

2. **App-facing abstractions**

   - `AppBuilder` (or `AppConfig`) to register:
     - views,
     - commands/actions,
     - keybindings,
     - global options (enable/disable status bar, help, command palette).
   - `View` (a screen: DAGs, Servers, Jobs, etc.).
   - Optional internal `Module` trait (plugin-like structure for grouping views and actions inside a single app, but still statically linked).
   - `Action` / `ActionId` (optional higher-level abstraction to decouple keybindings from operations).

3. **Standard widgets**

   - `GridView` (paginated list with columns).
   - `DetailView` (pane showing details for a selected item).
   - `LogView` (scrollable/streaming logs).
   - `ModalView` (confirmations, error details).
   - `StatusBar`.

The app author can:

- Use everything “by hand” (just call `AppBuilder` directly), or
- Create internal “modules” that implement a small trait and get registered by the main app, to mimic plugins without dynamic loading.

---

## 2. Execution model (v1)

For **v1**, the framework uses a **single-threaded, synchronous event loop**:

- The loop:
  - reads terminal input,
  - updates internal state,
  - calls `render` on the active view,
  - processes timers/refresh triggers.
- I/O (HTTP, gRPC clients, database, etc.) happens **inside handlers** and is **blocking** from the framework’s point of view.

Why:

- Much simpler mental model and implementation.
- Easy to embed in a CLI that might also spawn a REST/gRPC server (the CLI code can decide how to run the server: separate thread, async runtime, etc.).

**Future (v2+)**: background jobs and async integration (Tokio, non-blocking refreshes, streaming logs) can be added later, but the initial API should not make this impossible.

---

## 3. App and builder API

The main entrypoint for the application is an `AppBuilder`:

```rust
pub struct AppBuilder {
    // simplified; real type will hold registries and configuration
}

impl AppBuilder {
    pub fn new() -> Self { /* ... */ }

    pub fn with_status_bar(mut self, enabled: bool) -> Self { /* ... */ }
    pub fn with_help_overlay(mut self, enabled: bool) -> Self { /* ... */ }
    pub fn with_command_palette(mut self, enabled: bool) -> Self { /* ... */ }

    pub fn register_view<V: View + 'static>(mut self, view: V) -> Self { /* ... */ }

    pub fn map_view_slot(mut self, slot: ViewSlot, view_id: &'static str) -> Self { /* ... */ }

    pub fn configure_keymap(mut self, keymap: KeymapConfig) -> Self { /* ... */ }

    pub fn build(self, ctx: AppContext) -> App { /* ... */ }
}
```

The **application**:

- Defines its `AppContext` (service clients, configuration, etc.).
- Defines one or more `View` implementations.
- Registers views and keybindings via `AppBuilder`.
- Decides whether to enable/disable status bar, help overlay, command palette.
- Optionally starts a REST/gRPC server in parallel, using whatever runtime it wants.

### 3.1 Optional internal modules

If the app author wants a plugin-like structure inside the binary, we can provide a simple trait:

```rust
pub trait Module {
    fn id(&self) -> &'static str;
    fn register(&self, builder: &mut AppBuilder);
}
```

This is purely **static composition**:

- No dynamic loading.
- No multi-tenant routing.
- Just a way for the app to split “Airflow area”, “Hetzner area”, etc., within the same CLI.

---

## 4. `View` trait

A `View` represents a “screen”:

```rust
pub trait View {
    /// Stable identifier for this view. Literal, compile-time string.
    fn id(&self) -> &'static str;

    /// Name shown in the status bar / tabs.
    fn title(&self) -> &'static str;

    /// Called every frame to draw this view.
    fn render(&mut self, f: &mut Frame, area: Rect, ctx: &AppContext);

    /// Handles view-specific events (arrows, enter, letters, etc.).
    fn handle_event(&mut self, event: &Event, ctx: &mut AppContext) -> ViewResult;

    /// Help items specific to this view (used by '?').
    fn help_items(&self) -> Vec<HelpItem>;
}
```

Framework responsibilities:

- Call `render` on the active view each frame.
- Route keyboard events:
  - Apply **global keybindings** first (F1..F12, `?`, `:`, `q`).
  - Then call `handle_event` on the active view with remaining events.
- When the user presses `?`, merge:
  - global help (F1, F2, F3, `:`, `q`, etc.),
  - `help_items()` from the current view.

The app author only needs to implement concrete `View`s and register them.

---

## 5. DataSource, GridView and pagination

We want a **generic `GridView` widget** backed by a `DataSource` that:

- Supports **pagination** when the backend supports it.
- Falls back to **full in-memory list** when the backend does not.

### 5.1 `DataSource` trait

```rust
pub trait DataSource {
    type Row;

    /// Total number of rows (logical length).
    fn len(&self) -> usize;

    /// Access a row by index (0-based). Behind the scenes this may fetch a page.
    fn get(&self, index: usize) -> Option<&Self::Row>;

    /// Refresh underlying data (may fetch from network, disk, etc.).
    fn refresh(&mut self, ctx: &AppContext) -> anyhow::Result<()>;
}
```

Notes:

- A **non-paginated** backend can:
  - fetch everything once into a `Vec<Row>`,  
  - implement `len()` and `get()` over that vector.
- A **paginated** backend can:
  - keep an internal page cache,
  - load the right page when `get()` is called for an index not in cache.

### 5.2 `GridView`

```rust
pub struct GridView<D: DataSource> {
    data_source: D,
    columns: Vec<ColumnSpec<D::Row>>,
    selected: usize,
    // internal scroll/pagination state
}
```

Responsibilities:

- Handle keyboard navigation (up/down, page up/down, home/end).
- Keep track of the `selected` index.
- Render only the visible slice of rows based on terminal height.
- Delegate data loading to `DataSource::refresh`.

Example for an Airflow-like data source:

```rust
struct DagsDataSource {
    dags: Vec<DagSummary>,
    // maybe paging info, client, etc.
}

impl DataSource for DagsDataSource {
    type Row = DagSummary;

    fn len(&self) -> usize { self.dags.len() }

    fn get(&self, index: usize) -> Option<&DagSummary> {
        self.dags.get(index)
    }

    fn refresh(&mut self, ctx: &AppContext) -> anyhow::Result<()> {
        self.dags = ctx.clients.airflow.list_dags()?;
        Ok(())
    }
}
```

Then the view:

```rust
pub struct DagsView {
    grid: GridView<DagsDataSource>,
}

impl View for DagsView {
    fn id(&self) -> &'static str { "airflow.dags" }
    fn title(&self) -> &'static str { "DAGs" }

    fn render(&mut self, f: &mut Frame, area: Rect, ctx: &AppContext) {
        self.grid.render(f, area, ctx);
    }

    fn handle_event(&mut self, ev: &Event, ctx: &mut AppContext) -> ViewResult {
        self.grid.handle_event(ev, ctx)
    }

    fn help_items(&self) -> Vec<HelpItem> {
        vec![
            HelpItem::new("Enter", "View DAG Runs"),
            HelpItem::new("t", "Trigger DAG"),
            HelpItem::new("p", "Pause/Unpause DAG"),
        ]
    }
}
```

---

## 6. Keymap and global vs per-view bindings

The framework defines a **global keymap** with “opinionated defaults”:

- `F1..F12` → **view slots** (`ViewSlot::F1..F12`).
- `?` → open help overlay.
- `:` → open command palette.
- `q` → go back / exit (depending on context).

The **application** uses `AppBuilder` to:

- Map view IDs to slots:

  ```rust
  builder
      .register_view(DagsView::new())
      .register_view(RunsView::new())
      .map_view_slot(ViewSlot::F1, "airflow.dags")
      .map_view_slot(ViewSlot::F2, "airflow.runs");
  ```

- Configure additional keybindings via a `KeymapConfig`:

  ```rust
  pub struct KeymapConfig {
      pub global: Vec<KeyBinding>,
      pub per_view: HashMap<&'static str, Vec<KeyBinding>>,
  }
  ```

  where `KeyBinding` maps a key (or key sequence) to either:

  - a **view switch** (`SwitchView(view_id)`), or
  - an **action** (`InvokeAction(action_id)`), or
  - handed directly to the view via `handle_event`.

Additional requirements:

- **Global shortcuts are the primary way to bind views to F1/F2/F3**:
  - This is how you say “F1 = this view”.
- **Keymap reconfiguration is part of v1**:
  - At least through Rust code (static configuration).
  - Loading from files (YAML/TOML) can be considered for v2.
- Views and modals **may override/suppress** global bindings if needed:
  - e.g., a confirmation modal might capture `q` to close itself instead of exiting the app.

---

## 7. Messages, status bar, help and modals

The framework provides a standard **message model**:

```rust
pub enum AppMessageKind {
    Info,
    Warning,
    Error,
}

pub struct AppMessage {
    pub kind: AppMessageKind,
    /// Short one-line text for the status bar.
    pub short: String,
    /// Optional detailed text for a modal.
    pub details: Option<String>,
}
```

### 7.1 Status bar

- **Enabled by default**, but **opt-out** via `AppBuilder::with_status_bar(false)`.
- Always shown when enabled, at the bottom of the screen.
- Displays:
  - current view title,
  - current context summary (if the app wants to show it),
  - hints for global keys (F1/F2/F3, `?`, `:`, `q`),
  - the **short** text of the latest `AppMessage` (if any).

### 7.2 Help overlay (`?`)

- **Enabled by default**, but **opt-out**.
- Shows:
  - global keybindings,
  - view-specific `help_items()`.
- The app can disable or customize it, but the default behavior is consistent across views.

### 7.3 Command palette (`:`)

- **Enabled by default**, but **opt-out** via `AppBuilder`.
- Lists available commands / actions, searchable by text.
- Uses the same `ActionId` / `AppCommand` model as keybindings (see next section).

### 7.4 Modals

- Used to show:
  - confirmations,
  - detailed error messages,
  - multi-line info.
- By default, a detailed `AppMessage` (`details: Some(...)`) opens a modal when requested, while the **short** text is shown in the status bar.

---

## 8. Actions and commands (v1 vs v2)

We distinguish two levels:

1. **v1 (simpler)**:
   - Views handle their own keys via `handle_event`.
   - They can directly perform operations using `AppContext` (call clients, update state, push `AppMessage`s).

2. **v2 (more structured)**:
   - Introduce a formal `Action` / `ActionRegistry`:

     ```rust
     pub type ActionId = &'static str;

     pub enum AppCommand {
         SwitchView(&'static str),
         InvokeAction(ActionId),
     }
     ```

   - Keybindings and command palette both produce `AppCommand`s, which the core dispatches.
   - Views can register actions and associate them with the current selection (e.g., trigger DAG on selected row).

For now, we **document the idea** of actions and `AppCommand`, but v1 can be implemented with direct `handle_event` and evolve later without breaking the core concepts.

---

## 9. `AppContext` and responsibility boundaries

`AppContext` is **owned by the application** and passed into views and data sources:

```rust
pub struct AppContext {
    // Defined by the app.
    // Typical fields might include:
    // - service clients (REST/gRPC),
    // - configuration,
    // - metrics collectors, etc.
}
```

Framework rules:

- The framework does **not** impose a specific notion of “environment” (prod/staging).
- It only assumes that `AppContext` is:
  - available to `View::render`,
  - mutable for event handlers and `DataSource::refresh`.

Multi-environment support (`prod`, `staging`, etc.) is **left to the application** for now:

- The app may add fields like `current_environment: String`, `clients: Clients`, etc.
- Later, we can introduce optional helpers for environment switching if needed.

---

## 10. Integration with REST / gRPC and service monitoring

The CLI is primarily a **monitoring tool** for a single service, but:

- The same binary may also serve a **REST** or **gRPC** API.
- The framework itself:
  - does not start or manage the server,
  - but should **not make it hard** to run such a server alongside the TUI.

Typical pattern:

- The app:
  - constructs `AppContext` with one or more clients to the service,
  - may start an HTTP or gRPC server in a separate thread or async runtime,
  - then runs the TUI `App` built by the framework.
- Views and data sources:
  - use the clients in `AppContext` to query state and trigger operations.

Future (v2+) questions:

- Should the framework offer helpers to:
  - cleanly shut down the server when the TUI exits,
  - expose TUI metrics/state via the REST/gRPC API?

For now, these are left to the application.

---

## 11. v1 scope vs future extensions

**v1 (minimum usable)**:

- Single-threaded, synchronous event loop.
- `AppBuilder` with:
  - view registration,
  - mapping of view slots (F1..F12),
  - configuration of global/per-view keybindings via code,
  - toggles for status bar, help overlay, command palette.
- `View` trait with static `id()` and `title()`.
- `DataSource` + `GridView`:
  - `len()`, `get(index)`, `refresh()`,
  - grid handles keyboard navigation and visible slice.
- `AppMessage` model with:
  - messages shown in status bar (short),
  - optional detailed modal.
- Standard UI elements:
  - status bar (opt-out),
  - help overlay (opt-out),
  - command palette (opt-out),
  - simple modal component.

**v2+ (explicitly future work)**:

- Background jobs / async refresh (non-blocking I/O).
- Formal `Action` / `ActionRegistry` and unified command model.
- Loading keymaps and configuration from external files (YAML/TOML).
- Split-screen / multiple views visible at once.
- More advanced logging (streaming, filters, search).
- Shared helpers for multi-environment support.
- Tighter integration helpers for embedded REST/gRPC servers.

---

## 12. Open questions (next design steps)

Some questions that remain open and are worth deciding in later iterations:

1. **Command palette model**  
   Do we want a formal `Command` abstraction (with description, arguments, categories), or is it just a list of actions plus some built-in commands?

2. **Standard command syntax**  
   Should we define a standard textual syntax (e.g. `:restart service=api env=prod`) that maps to commands, or leave it entirely to the application?

3. **Logs and streaming**  
   How far should v2 go in supporting streaming logs (e.g. tail -f style) and log filtering/search directly in `LogView`?

4. **Service health and metrics**  
   Do we want optional helpers or conventions for:
   - health checks,
   - metrics panels,
   - alert banners in the status bar?

5. **REST/gRPC lifecycle helpers**  
   Should the framework provide utilities to:
   - coordinate shutdown between TUI and REST/gRPC servers,
   - expose high-level TUI state via those APIs?

6. **Thread-safety and async**  
   Do we want to require `AppContext` (and main types) to be `Send + Sync` from day one, to make the later async/background job story smoother?

These questions do not block v1, but the answers will influence how far the framework can go in v2+ without breaking changes.

