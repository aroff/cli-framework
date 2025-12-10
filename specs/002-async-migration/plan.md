# Implementation Plan: Async Runtime Migration

**Branch**: `002-async-migration` | **Date**: 2025-12-09 | **Spec**: [spec.md](./spec.md)
**Input**: Feature specification from `/specs/002-async-migration/spec.md`

**Note**: This template is filled in by the `/speckit.plan` command. See `.specify/templates/commands/plan.md` for the execution workflow.

## Summary

Migrate cli-framework from synchronous to asynchronous runtime using Tokio. This enables direct integration with async services (e.g., FastSkill, HTTP APIs, databases) without blocking the UI during network operations. The framework will create and manage the Tokio runtime internally, convert all trait methods (DataSource, View, Command) to async, implement a background task system, and provide automatic loading indicators. This is a breaking change requiring a major version bump (0.1.0 → 0.2.0).

## Technical Context

**Language/Version**: Rust 1.75+ (edition 2021)  
**Primary Dependencies**: 
- `ratatui = "0.27"` (TUI rendering - remains sync)
- `crossterm = "0.28"` (terminal I/O - has async support)
- `tokio = "1.0"` (async runtime - NEW)
- `async-trait = "0.1"` (async trait support - NEW)
- `tokio-util = "0.7"` (async utilities - NEW)
- `anyhow = "1.0"` (error handling)
- `serde = "1.0"` (serialization)

**Storage**: N/A (framework library, no persistent storage)  
**Testing**: `cargo test` with unit tests, integration tests, and contract tests. TUI testing uses `ratatui::backend::TestBackend` for headless testing. Async tests use `#[tokio::test]`.  
**Target Platform**: Linux, macOS, Windows (terminal-based CLI applications)  
**Project Type**: Library crate (`tui-framework`) that applications statically link  
**Performance Goals**: 
- Event loop latency: ≤16ms per frame (SC-007)
- UI responsiveness: ≤50ms for user interactions during async operations (SC-002)
- Loading indicator appearance: ≤100ms after operation start (SC-011)
- Background task results: ≤100ms to appear in UI (SC-010)
- Streaming data updates: ≤100ms latency (SC-004)

**Constraints**: 
- Must maintain render performance (rendering remains sync, called from async context)
- Must support minimum terminal size 80x24
- Must be Send + Sync for thread safety
- Breaking change - major version bump required

**Scale/Scope**: 
- All existing examples must be migratable (3 examples)
- Support integration with 3+ common async Rust libraries (reqwest, tokio-postgres, tokio-fs)
- Framework manages Tokio runtime internally (no application setup required)

## Constitution Check

*GATE: Must pass before Phase 0 research. Re-check after Phase 1 design.*

### I. Library-First ✅
- Framework remains a standalone library crate
- All async features are self-contained
- Applications don't need to manage Tokio runtime (framework handles it)
- **Status**: Compliant

### II. Test-First (NON-NEGOTIABLE) ✅
- All async trait methods require contract tests
- Integration tests must verify async operations don't block UI
- Unit tests for async event loop, background tasks, cancellation
- **Status**: Compliant - TDD will be followed

### III. Documentation & API Design ✅
- All async trait methods must have doc comments with examples
- Migration guide required (FR-011)
- Examples must be updated to demonstrate async patterns
- **Status**: Compliant

### IV. Observability ✅
- Optional debug logging for async operations
- Compatible with OpenTelemetry (no changes needed)
- **Status**: Compliant

### V. Simplicity & Error Handling ✅
- Use `anyhow::Result` for async operations (already in use)
- Keep async patterns straightforward (use `async-trait` for clarity)
- Avoid over-engineering (use Tokio's built-in features)
- **Status**: Compliant

**Overall**: ✅ All gates pass. No violations.

## Project Structure

### Documentation (this feature)

```text
specs/002-async-migration/
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
├── app/
│   ├── mod.rs
│   ├── builder.rs       # AppBuilder - add async support
│   ├── context.rs       # AppContext - add Send + Sync bounds
│   ├── runtime.rs       # Runtime - convert to async event loop
│   └── background_tasks.rs  # NEW - Background task manager
├── view/
│   ├── mod.rs
│   ├── view_trait.rs    # View trait - convert handle_event to async
│   ├── registry.rs      # ViewRegistry - no changes needed
│   └── theme.rs         # Theme - no changes needed
├── data_source/
│   ├── mod.rs
│   ├── data_source_trait.rs  # DataSource trait - convert refresh to async
│   └── log.rs           # LogSource - may need async updates
├── command/
│   ├── mod.rs           # Command - convert execute to async
│   ├── registry.rs      # CommandRegistry - no changes needed
│   ├── parser.rs        # Command parser - no changes needed
│   └── palette.rs       # CommandPalette - no changes needed
├── widget/
│   ├── mod.rs
│   ├── grid.rs          # GridView - work with async DataSource
│   ├── log.rs           # LogView - async streaming support
│   ├── modal.rs         # ModalView - no changes needed
│   ├── status_bar.rs    # StatusBar - no changes needed
│   ├── help.rs          # HelpOverlay - no changes needed
│   ├── empty_state.rs   # EmptyState - no changes needed
│   └── view_header.rs  # ViewHeader - no changes needed
├── keymap/              # No changes needed
├── message/             # No changes needed
├── retry/               # May need async updates
└── lib.rs               # Update exports, add tokio dependency docs

examples/
├── simple/              # Update to async
├── multi_view/          # Update to async
└── kitchen_sink/        # Update to async

tests/
├── contract/            # Add async contract tests
├── integration/         # Add async integration tests
└── unit/                # Add async unit tests
```

**Structure Decision**: Single library crate structure maintained. New `background_tasks.rs` module added for background task management. All existing modules updated to support async where needed.

## Complexity Tracking

> **Fill ONLY if Constitution Check has violations that must be justified**

No violations - all gates pass. Async migration adds complexity but is justified by:
- Enabling direct integration with modern async Rust services
- Improving user experience (non-blocking UI)
- Aligning with ecosystem standards
- Simpler alternative (sync with blocking bridges) rejected due to poor UX and integration complexity
