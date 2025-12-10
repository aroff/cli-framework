# Implementation Plan: Progress Reporting for CLI Applications

**Branch**: `004-progress-reporting` | **Date**: 2025-01-27 | **Spec**: [spec.md](./spec.md)
**Input**: Feature specification from `/specs/004-progress-reporting/spec.md`

**Note**: This template is filled in by the `/speckit.plan` command. See `.specify/templates/commands/plan.md` for the execution workflow.

## Summary

Add structured progress reporting capabilities to the CLI framework, enabling applications to provide real-time user feedback during long-running operations. The feature extends the existing `BackgroundTaskManager` with progress update channels, allowing background tasks to send progress updates (current count, total count, optional messages) that are formatted and displayed to users in the terminal. Progress updates support in-place line overwriting, graceful degradation for indeterminate progress, and configurable display strategies for concurrent operations.

## Technical Context

**Language/Version**: Rust 1.75+ (edition 2021)  
**Primary Dependencies**: 
- `tokio` 1.0 (async runtime, already in use)
- `tokio::sync::mpsc` (for progress update channels)
- `ratatui` 0.27 (TUI rendering, already in use)
- `crossterm` 0.28 (terminal I/O, already in use)
- `anyhow` 1.0 (error handling, already in use)

**Storage**: N/A (in-memory progress state only)  
**Testing**: `cargo test` with unit tests, integration tests, and contract tests for progress reporting API  
**Target Platform**: Linux, macOS, Windows (terminal-based CLI applications)  
**Project Type**: Library crate (`tui-framework`) - extension to existing framework  
**Performance Goals**: 
- Progress updates displayed within 100ms of operation step completion (SC-001)
- <5% performance degradation compared to operations without progress reporting (SC-003)
- Support 1 to 1,000,000 items (SC-002)
- Support up to 100 concurrent operations (SC-004)

**Constraints**: 
- Must be opt-in (FR-009) - existing functionality unaffected
- Non-blocking progress updates (FR-010)
- Best-effort delivery (progress updates should not prevent operations from completing)
- Text-based output only (no visual progress bars per Out of Scope)

**Scale/Scope**: 
- Library extension to existing `BackgroundTaskManager`
- New module: `src/app/background_tasks.rs` (extend existing)
- New module: `src/cli_output.rs` (new formatting utilities)
- Integration with existing runtime and message systems

## Constitution Check

*GATE: Must pass before Phase 0 research. Re-check after Phase 1 design.*

### I. Library-First ✅
- **Status**: PASS
- **Rationale**: Feature extends existing library crate without adding hosting code. Progress reporting is a self-contained capability that applications opt into.

### II. Test-First (NON-NEGOTIABLE) ✅
- **Status**: PASS
- **Rationale**: TDD will be followed. Contract tests required for progress reporting API. Unit tests for `ProgressReporter` and formatting functions. Integration tests for end-to-end progress display.

### III. Documentation & API Design ✅
- **Status**: PASS
- **Rationale**: All public API (ProgressReporter, spawn_with_progress, formatting functions) must have doc comments with examples. Examples directory will demonstrate progress reporting usage.

### IV. Observability ✅
- **Status**: PASS
- **Rationale**: Progress reporting is opt-in and does not require observability. Framework remains lightweight. Optional debug logging can be added for progress update delivery.

### V. Simplicity & Error Handling ✅
- **Status**: PASS
- **Rationale**: Uses existing `tokio::sync::mpsc` patterns already in framework. Simple channel-based communication. Error handling via `anyhow::Result`. No complex state machines or additional dependencies.

**Gate Status**: ✅ ALL GATES PASS - Proceed to Phase 0

## Project Structure

### Documentation (this feature)

```text
specs/004-progress-reporting/
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
│   └── background_tasks.rs    # Extend with progress reporting (spawn_with_progress, ProgressReporter)
├── cli_output.rs              # NEW: Progress formatting and display utilities
└── lib.rs                     # Export new modules

tests/
├── unit/
│   ├── progress_reporter.rs   # Unit tests for ProgressReporter
│   └── cli_output.rs          # Unit tests for formatting functions
├── integration/
│   └── progress_reporting.rs  # Integration tests for end-to-end progress display
└── contract/
    └── progress_api.rs        # Contract tests for progress reporting API

examples/
└── progress_demo/             # NEW: Example demonstrating progress reporting
    └── main.rs
```

**Structure Decision**: Single library crate structure. Feature extends existing `src/app/background_tasks.rs` and adds new `src/cli_output.rs` module. No new top-level directories needed. Tests follow existing pattern in `tests/` directory.

## Complexity Tracking

> **No violations - all gates passed. Complexity is justified by feature requirements.**
