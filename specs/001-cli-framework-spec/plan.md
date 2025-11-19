# Implementation Plan: CLI Framework – Opinionated TUI Library

**Branch**: `001-cli-framework-spec` | **Date**: 2025-11-17 | **Spec**: [spec.md](./spec.md)
**Input**: Feature specification from `/specs/001-cli-framework-spec/spec.md`

**Note**: This template is filled in by the `/speckit.plan` command. See `.specify/templates/commands/plan.md` for the execution workflow.

## Summary

Build an opinionated TUI framework library in Rust that minimizes hosting code for CLI developers. The framework provides a complete event loop, layout system, navigation, status bar, help overlay, command palette, and standard widgets (GridView, LogView, ModalView) so application authors can focus on implementing views, datasources, and commands rather than terminal management.

**Technical Approach**: Rust-based library using `ratatui` for rendering and `crossterm` for terminal I/O. Single-threaded synchronous event loop for v1, with opinionated defaults for keybindings (F1-F12 view slots, `?` for help, `:` for command palette) and optional features (authentication, observability via OpenTelemetry).

## Technical Context

**Language/Version**: Rust (latest stable, minimum 1.75+)  
**Primary Dependencies**: 
- `ratatui` (TUI rendering library)
- `crossterm` (terminal I/O and event handling)
- `anyhow` (error handling)
- `serde` (serialization for configuration)
- OpenTelemetry crates (optional observability integration):
  - `opentelemetry` (core OpenTelemetry API)
  - `opentelemetry-otlp` (OTLP exporter for metrics/traces)
  - `opentelemetry-sdk` (OpenTelemetry SDK, optional for applications)

**Storage**: N/A (in-memory state only; applications manage their own data persistence via AppContext)

**Testing**: `cargo test` with unit tests, integration tests, and contract tests for View/DataSource traits.
- **TUI Testing Strategy**: Use `ratatui::backend::TestBackend` for headless testing of rendering logic. This allows asserting the buffer content (characters and styles) at specific coordinates without a physical terminal.
- **Reference Application**: `examples/kitchen_sink` will implement all features (auth, grid, logs, commands) to serve as the primary integration test bench.

**Target Platform**: Linux, macOS, Windows (terminal-based CLI applications)

**Project Type**: Library crate (`tui-framework`) that applications statically link

**Performance Goals**: 
- Screen updates and navigation within ~1 second for typical operations
- Handle thousands of rows in grids and tens of thousands of log lines efficiently
- Responsive UI during network operations (with retry/timeout handling)

**Constraints**: 
- Single-threaded synchronous execution model for v1
- Minimum terminal size: 80x24 characters with graceful degradation
- Must support opt-in authentication, retry policies, and OpenTelemetry observability
- Views persist state when switching (scroll position, selection, filters)

**Scale/Scope**: 
- Single service per binary (no multi-tenant plugin hosting)
- Typical use cases: thousands of rows, tens of thousands of log lines
- Applications can register multiple views (F1-F12 slots), commands, and keybindings

## Constitution Check

*GATE: Must pass before Phase 0 research. Re-check after Phase 1 design.*

**Note**: Constitution file at `.specify/memory/constitution.md` is customized (Version 1.0.0, ratified 2025-11-17). The following gates are derived from the constitution's core principles.

**Gates**:
- ✅ Library-first: Framework is a standalone, independently testable library crate (Constitution I)
- ✅ Test-first: TDD mandatory - tests written before implementation, contract tests for all core abstractions (Constitution II - NON-NEGOTIABLE)
- ✅ Documentation: Public API must be documented with examples (Constitution III)
- ✅ Error handling: Use `anyhow::Result` for framework operations (Constitution V)
- ✅ Observability: Optional OpenTelemetry integration for debugging (Constitution IV)

## Project Structure

### Documentation (this feature)

```text
specs/001-cli-framework-spec/
├── plan.md              # This file (/speckit.plan command output)
├── research.md          # Phase 0 output (/speckit.plan command)
├── data-model.md        # Phase 1 output (/speckit.plan command)
├── quickstart.md        # Phase 1 output (/speckit.plan command)
├── contracts/           # Phase 1 output (/speckit.plan command)
└── tasks.md             # Phase 2 output (/speckit.tasks command - NOT created by /speckit.plan)
```

### Source Code (repository root)

```text
src/
├── lib.rs               # Library root, re-exports public API
├── app/
│   ├── mod.rs
│   ├── builder.rs       # AppBuilder implementation
│   ├── context.rs       # AppContext (application-defined, framework provides trait/helpers)
│   └── runtime.rs       # Event loop, view routing
├── view/
│   ├── mod.rs
│   ├── trait.rs         # View trait definition
│   ├── registry.rs       # ViewRegistry for managing views
│   └── state.rs         # View state persistence
├── widget/
│   ├── mod.rs
│   ├── grid.rs          # GridView implementation
│   ├── log.rs           # LogView implementation
│   ├── modal.rs         # ModalView implementation
│   ├── status_bar.rs    # StatusBar widget
│   └── empty_state.rs   # Empty state and loading indicators
├── data_source/
│   ├── mod.rs
│   └── trait.rs         # DataSource trait definition
├── command/
│   ├── mod.rs
│   ├── registry.rs      # CommandRegistry
│   ├── parser.rs        # Command syntax parser (:command arg=value)
│   └── palette.rs       # Command palette UI
├── keymap/
│   ├── mod.rs
│   ├── config.rs        # KeymapConfig
│   ├── registry.rs      # KeymapRegistry
│   └── resolver.rs      # Conflict resolution (view > global, modal > all)
├── message/
│   ├── mod.rs
│   └── model.rs         # AppMessage, AppMessageKind
├── auth/
│   ├── mod.rs
│   ├── login.rs         # Login screen (optional)
│   ├── token.rs         # Token management (optional)
│   └── rbac.rs          # Role-based access control (optional)
├── retry/
│   ├── mod.rs
│   ├── policy.rs        # RetryPolicy configuration
│   └── executor.rs      # Retry execution logic
└── observability/
    ├── mod.rs
    └── opentelemetry.rs # OpenTelemetry integration (optional)

tests/
├── unit/
│   ├── view/
│   ├── widget/
│   ├── command/
│   └── keymap/
├── integration/
│   ├── app_builder.rs
│   ├── view_switching.rs
│   └── command_palette.rs
└── contract/
    ├── view_trait.rs    # Contract tests for View implementations
    └── data_source_trait.rs # Contract tests for DataSource implementations

examples/
├── simple/
│   └── main.rs          # Minimal example: one view, one datasource
├── multi_view/
│   └── main.rs          # Multiple views with F-key mapping
└── with_commands/
    └── main.rs          # Example with commands and command palette
```

**Structure Decision**: Single library crate structure. The framework is organized by domain (app, view, widget, command, etc.) with clear separation of concerns. Tests are organized by type (unit, integration, contract) to ensure comprehensive coverage of the framework's abstractions.

## Complexity Tracking

> **Fill ONLY if Constitution Check has violations that must be justified**

No violations identified. The framework follows standard Rust library patterns with clear abstractions and minimal complexity.

## Phase Completion Status

### Phase 0: Research ✅

**Completed**: 2025-11-17

- Technology choices documented (Rust, ratatui, crossterm, OpenTelemetry)
- Architecture patterns decided (Builder pattern, trait-based abstractions)
- Keybinding and command patterns defined
- Error handling and resilience patterns established
- UI/UX patterns documented

**Output**: `research.md`

### Phase 1: Design & Contracts ✅

**Completed**: 2025-11-17

- Data model extracted and documented
- API contracts defined for core traits (View, DataSource, AppBuilder)
- Quickstart guide created
- Agent context updated

**Outputs**:
- `data-model.md`
- `contracts/view-trait.md`
- `contracts/data-source-trait.md`
- `contracts/app-builder-api.md`
- `quickstart.md`
- `.cursor/rules/specify-rules.mdc` (agent context)

### Phase 2: Task Breakdown

**Status**: Pending (run `/speckit.tasks` to generate)

**Next Steps**:
1. Run `/speckit.tasks` to break down implementation into concrete tasks
2. Begin implementation following the task breakdown
3. Write tests alongside implementation (TDD approach)
