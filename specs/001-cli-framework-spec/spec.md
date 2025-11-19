# Feature Specification: CLI Framework – Opinionated TUI Library

**Feature Branch**: `001-cli-framework-spec`  
**Created**: 2025-11-17  
**Status**: Draft  
**Input**: User description: "please, specify project cli-framework.@cli-framework.md on /home/sysuser/ws001/cli-framework"

## Clarifications

### Session 2025-11-17

- Q: Should the framework provide built-in authentication/authorization mechanisms, or is this entirely the application's responsibility? → A: Framework provides built-in authentication mechanisms (e.g., login screen, token management, role-based access control), but applications opt-in to use them when required.
- Q: What are the expected data volume limits the framework should handle efficiently (e.g., maximum rows in a grid, maximum log lines in memory)? → A: No hard limits; optimize for typical use cases (thousands of rows, tens of thousands of log lines).
- Q: Should the framework provide built-in retry logic and timeout handling for network operations, or should applications handle this in their clients/AppContext? → A: Framework provides configurable retry policies and timeout handling for all network operations.
- Q: Should the framework provide built-in observability (logging, metrics, tracing) for framework operations, or is debugging left entirely to application-level logging? → A: Framework provides optional debug logging/metrics that applications can enable, and must be compatible with OpenTelemetry.
- Q: When a keybinding conflict occurs (global vs. view-specific, or multiple views), what is the resolution priority order? → A: View-specific bindings override global; modals override everything.
- Q: Should views be created once and persist their state when switching between them, or should they be recreated each time they're activated? → A: Views are created once at startup and persist their state (scroll position, selection, filters) when switching between views.
- Q: What is the minimum terminal size the framework should support, and how should it handle terminals smaller than that? → A: Minimum 80x24; gracefully degrade with scroll/truncation for smaller terminals.
- Q: Should the framework provide standard empty state messages and loading indicators, or should applications handle these in their views? → A: Framework provides mandatory standard empty state messages and loading indicators for all views.
- Q: Should the framework define a specific default color scheme, or should it attempt to adapt to the terminal's existing colors? → A: Use standard ANSI colors (16-color palette) by default to respect terminal user preferences.

## User Scenarios & Testing *(mandatory)*

### User Story 1 - Build a TUI console for my service quickly (Priority: P1)

A backend/platform engineer wants to build a TUI console for a specific service (e.g., Airflow, Hetzner, internal control plane) and focus on **commands and operations**, not on terminal event loops, layouts, or key handling.  
They include the CLI framework library, configure a few views and datasources, and get a working, consistent TUI with status bar, help, command palette, and grid/detail/log views with minimal code.

**Why this priority**: This is the **primary value** of the framework: drastically reducing the effort to stand up a robust TUI console for a single service.

**Independent Test**:  
Given an example service (e.g., a demo REST API with some listable resources), a developer can:

- wire a small number of views and datasources using the framework, and  
- obtain a TUI that supports navigation, help, command palette, and basic grid/log views  

without writing their own event loop, layout, or key dispatch logic.

**Acceptance Scenarios**:

1. **Given** a new CLI project and the framework library, **When** the developer registers at least one `View` and maps it to `F1`, **Then** they can launch the CLI and see that view rendered with a status bar and help available via `?`.
2. **Given** a `GridView` wired to a `DataSource` that calls a demo service, **When** the developer runs the CLI, **Then** they can navigate rows with arrow keys and page keys and see data updated when `refresh` is triggered.

---

### User Story 2 - Operate a service via commands and keybindings (Priority: P1)

An operator uses the TUI console to **monitor and act on** the service: listing items (jobs, servers, DAGs, etc.), selecting them, and triggering operations (restart, scale, trigger run, clear task).  
They use both keybindings (e.g., `t` to trigger, `p` to pause) and the command palette (e.g., `:restart service=api env=prod`) to execute operations.

**Why this priority**: The framework must not only show data but also make **operations discoverable and repeatable** via keyboard and commands, otherwise operators will fall back to raw CLI commands or dashboards.

**Independent Test**:

- Implement a small set of operations on top of a demo service (e.g., “trigger job”, “restart worker”), register them as commands and/or actions, and verify they can be invoked via keys and palette without needing to know raw HTTP/CLI details.

**Acceptance Scenarios**:

1. **Given** a `GridView` with selectable rows representing service entities, **When** the user presses a configured keybinding (e.g., `t`), **Then** the framework routes this to the appropriate operation, and the user receives feedback via status bar/modal.
2. **Given** a set of registered commands with syntax hints, **When** the user opens the command palette with `:` and types `:restart service=api env=prod`, **Then** the console runs the corresponding command and shows success/failure via `AppMessage`.

---

### User Story 3 - Inspect live logs and filter issues (Priority: P2)

An operator wants to see **live streaming logs** from the service within the TUI, without switching to a separate `logs --follow` CLI session.  
They open a `LogView`, enable follow mode, and apply a simple keyword filter (e.g., `error` or a request ID) to focus on relevant lines.

**Why this priority**: Being able to **observe logs in context** (alongside views and commands) improves troubleshooting speed, especially when operations (like a restart) and resulting logs are closely related.

**Independent Test**:

- Connect `LogView` to a demo log source that emits lines over time; verify follow mode and keyword filtering work independently of other views.

**Acceptance Scenarios**:

1. **Given** a `LogView` connected to a streaming log source, **When** new lines arrive, **Then** they appear in the buffer and are visible when follow mode is enabled.
2. **Given** a filter string (e.g., `error`), **When** the user applies it in `LogView`, **Then** only lines containing that substring (case-insensitive) are displayed.

---

### User Story 4 - Customize keybindings and UI features (Priority: P3)

A power user or platform team wants to configure **which views are on F-keys**, adjust global and per-view keybindings, and optionally disable the status bar, help overlay, or command palette to better fit their environment or preferences.

**Why this priority**: Opinionated defaults are useful, but teams often have established conventions or accessibility needs that require adjusting keys and visible UI chrome.

**Independent Test**:

- Configure an application to use non-default view mappings for F-keys, add a custom keybinding for a command, and disable the status bar; verify these changes are applied without modifying the framework internals.

**Acceptance Scenarios**:

1. **Given** a configuration that maps `F1` and `F2` to different registered views, **When** the user presses these keys, **Then** the corresponding views are shown without changing framework code.
2. **Given** a configuration that disables the status bar or help overlay, **When** the user runs the CLI, **Then** those elements are not shown while the application remains fully functional.

---

### Edge Cases

- What happens when the service or network is unavailable (e.g., refresh fails, command call fails): The framework applies configured retry policies and timeout handling, then surfaces errors to the user via status bar and/or modal using AppMessage.
- How the framework behaves when terminals are very small: The framework supports a minimum terminal size of 80x24 characters and gracefully degrades with scroll/truncation for smaller terminals, ensuring all core functionality remains accessible.
- How keybinding conflicts are resolved when both global keymap and a specific view want the same key: View-specific bindings override global bindings; modals override everything (including view-specific bindings).

## Requirements *(mandatory)*

### Functional Requirements

- **FR-001**: The framework MUST provide an `AppBuilder` that allows applications to register views, map views to F1–F12 slots, configure keybindings, and toggle core UI features (status bar, help overlay, command palette).
- **FR-002**: The framework MUST define a `View` abstraction that supports rendering, handling input events, and exposing help items, so that applications can implement views without managing the event loop directly. Views MUST be created once at startup and persist their state (scroll position, selection, filters) when switching between views.
- **FR-003**: The framework MUST provide a paginated `GridView` backed by a `DataSource` trait that supports both full in-memory lists and lazy page loading through a uniform API.
- **FR-004**: The framework MUST provide a `LogView` that can display streaming log lines from an application-provided source, with scrolling, follow mode, and a single keyword filter per line (case-insensitive).
- **FR-005**: The framework MUST provide a standard message model (`AppMessage`) and UI elements (status bar and modal) to surface short and detailed feedback for operations and errors.
- **FR-013**: The framework MUST provide mandatory standard empty state messages and loading indicators for all views to ensure consistent UX when data is unavailable or being loaded.
- **FR-014**: The framework MUST use standard ANSI colors (16-color palette) for its default theme to ensure compatibility with user terminal preferences and readability across different terminal color schemes.
- **FR-006**: The framework MUST provide a global keymap with opinionated defaults (F1–F12 view slots, `?` for help, `:` for command palette, `q` for back/exit) and allow applications to customize global and per-view bindings in code. When keybinding conflicts occur, view-specific bindings MUST override global bindings, and modals MUST override all other bindings.
- **FR-007**: The framework MUST provide a formal `Command` abstraction and a command palette that supports listing commands and executing them via a standard textual syntax (e.g., `:restart service=api env=prod`).
- **FR-008**: The framework MUST allow applications to integrate their own service clients and optional REST/gRPC servers via an `AppContext` object without imposing specific environment or deployment assumptions.
- **FR-009**: The framework MUST support building CLIs that focus on a single service per binary, with optional internal modularization (e.g., modules grouping related views), but without multi-tenant plugin hosting.
- **FR-010**: The framework MUST provide optional built-in authentication mechanisms (login screen, token management, role-based access control) that applications can opt-in to use when required, while remaining security-agnostic for applications that handle authentication via AppContext.
- **FR-011**: The framework MUST provide configurable retry policies and timeout handling for network operations (e.g., DataSource refresh, Command execution) to ensure robust error handling and user feedback during transient failures.
- **FR-012**: The framework MUST provide optional debug logging and metrics that applications can enable for troubleshooting framework behavior, and MUST be compatible with OpenTelemetry for integration with standard observability tooling.

### Key Entities *(include if feature involves data)*

- **View**: Represents a screen within the TUI (e.g., list of jobs, details of a resource, log stream), exposes an identifier, title, render method, event handler, and help items.
- **DataSource**: Represents a provider of tabular data for `GridView`, capable of reporting logical length, retrieving rows by index, and refreshing data from the underlying service.
- **Command**: Represents an executable operation (such as restarting a service or triggering a job) with an identifier, summary, optional syntax hint, category, and an execution function that receives parsed arguments.
- **AppContext**: Represents application-owned state and service clients (such as REST/gRPC clients, configuration, and metrics sources) that views, datasources, and commands use to interact with the service.
- **AppMessage**: Represents user-visible messages with a kind (info/warning/error), a short text for the status bar, and optional detailed text for a modal.

## Success Criteria *(mandatory)*

### Measurable Outcomes

- **SC-001**: A developer familiar with the service but new to the framework can build a basic multi-view TUI console (at least one grid view and one log view) in **less than one working day** using the framework’s abstractions.
- **SC-002**: Operators can complete core operations (e.g., listing entities and triggering an action on a selected item) within **three keyboard interactions** from the main screen (for example, F1 → navigate → keybinding/command).
- **SC-003**: In usability tests with at least three practitioners, **80% or more** report that the framework-based TUI is easier to use than a raw CLI for the same operations (subjective satisfaction).
- **SC-004**: For representative services and data sizes (thousands of rows in grids, tens of thousands of log lines), the TUI remains responsive (screen updates and navigation within ~1 second) while performing typical refreshes and log streaming, as perceived by users during testing. The framework optimizes for these typical use cases without imposing hard limits.