# Implementation Plan: CLI Mode Detection

**Branch**: `009-cli-mode-detection` | **Date**: 2025-01-27 | **Spec**: [../009_cli_mode_detection.md](../009_cli_mode_detection.md)
**Input**: Feature specification from `/specs/009_cli_mode_detection.md`

**Note**: This template is filled in by the `/speckit.plan` command. See `.specify/templates/commands/plan.md` for the execution workflow.

## Summary

Provide comprehensive utilities for detecting CLI execution context and adapting application behavior accordingly. The feature enables applications to automatically detect whether they are running in an interactive terminal, determine output preferences (colors, format), and adapt behavior for interactive vs non-interactive environments.

**Technical Approach**: Create a new `cli_mode` module in the Rust library that provides detection utilities for terminal type (TTY checks for stdin/stdout/stderr), interactive mode, color output preferences, output format selection, progress indicator display, quiet mode, and optional terminal dimensions. The module uses standard library `std::io::IsTerminal` trait (Rust 1.70+) for reliable cross-platform TTY detection, respects standard environment variables (NO_COLOR, OUTPUT_FORMAT, QUIET, FORCE_COLOR), and returns safe defaults when detection fails.

## Technical Context

**Language/Version**: Rust 1.75+ (edition 2021, requires Rust 1.70+ for `std::io::IsTerminal`)  
**Primary Dependencies**: 
- Standard library only (`std::io::IsTerminal` for TTY detection)
- `std::env` for environment variable access
- Optional: `terminal_size` crate (feature-gated) for terminal dimension detection

**Storage**: N/A (stateless detection utilities, no persistent state)

**Testing**: `cargo test` with unit tests for all detection functions. Test cases include:
- TTY detection for stdin/stdout/stderr in various combinations
- Environment variable precedence and conflict resolution
- Error handling and safe defaults when detection fails
- Cross-platform compatibility (Windows, Linux, macOS)
- Piped output detection
- Terminal dimension detection (when available)
- Mixed stream states (different TTY states for different streams)

**Target Platform**: Cross-platform (Linux, macOS, Windows) - CLI framework

**Project Type**: Library crate module (`tui_framework::cli_mode`) that applications opt-in to use

**Performance Goals**: 
- All detection functions complete in <1ms (stateless checks, no I/O)
- No performance impact on application startup or runtime

**Constraints**: 
- Must use `std::io::IsTerminal` trait (Rust 1.70+) for cross-platform TTY detection
- Must respect standard environment variables (NO_COLOR, OUTPUT_FORMAT, QUIET, FORCE_COLOR)
- Must return safe defaults (non-interactive mode) when detection fails
- Must support independent detection for stdin/stdout/stderr streams
- Must maintain backward compatibility with existing code (new module, opt-in usage)
- Environment variable precedence: NO_COLOR > FORCE_COLOR for colors; OUTPUT_FORMAT > terminal-based defaults

**Scale/Scope**: 
- New module: `src/cli_mode/mod.rs`
- Stateless utility functions (no structs or state management)
- Integration with existing `cli_output` module (may refactor existing color detection)
- Optional terminal dimension detection (may return None)

## Constitution Check

*GATE: Must pass before Phase 0 research. Re-check after Phase 1 design.*

### I. Library-First ✅
- **Status**: PASS
- **Rationale**: New `cli_mode` module is self-contained within the library crate. Applications opt-in by using the utilities. No hosting code required.

### II. Test-First (NON-NEGOTIABLE) ✅
- **Status**: PASS
- **Rationale**: All detection functions will have unit tests written first. Test cases cover all functional requirements, edge cases, and cross-platform scenarios.

### III. Documentation & API Design ✅
- **Status**: PASS
- **Rationale**: All public functions will have doc comments with examples. The module will be documented in the library documentation. API follows Rust naming conventions.

### IV. Observability ✅
- **Status**: PASS
- **Rationale**: Detection functions return simple boolean/option types. Optional debug logging available for troubleshooting detection failures. No complex observability requirements.

### V. Simplicity & Error Handling ✅
- **Status**: PASS
- **Rationale**: Straightforward detection utilities. Functions return safe defaults (non-interactive mode) when detection fails, avoiding error propagation. No complex abstractions needed.

**Gate Result**: ✅ **PASS** - All constitution principles satisfied.

## Project Structure

### Documentation (this feature)

```text
specs/009-cli-mode-detection/
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
├── cli_mode/            # New module for CLI mode detection
│   ├── mod.rs           # Module root, public API
│   └── detection.rs     # Core detection functions (optional, if module grows)
└── lib.rs               # Add cli_mode module re-export

tests/
├── cli_mode/            # Unit tests for CLI mode detection
│   ├── mod.rs
│   ├── tty_detection.rs # Tests for TTY detection
│   ├── color_detection.rs # Tests for color detection
│   ├── format_detection.rs # Tests for format detection
│   ├── interactive_mode.rs # Tests for interactive mode
│   ├── quiet_mode.rs    # Tests for quiet mode
│   ├── terminal_dimensions.rs # Tests for terminal dimensions (optional)
│   └── environment_variables.rs # Tests for env var handling
└── integration/
    └── cli_mode_integration.rs # Integration tests
```

**Structure Decision**: Create new `cli_mode` module. Functions are stateless utilities, so a single `mod.rs` file is sufficient initially. If the module grows, we can split into submodules. Integration with existing `cli_output` module may require refactoring existing `should_use_color()` function to use new `cli_mode` utilities.

## Complexity Tracking

> **Fill ONLY if Constitution Check has violations that must be justified**

N/A - No violations detected.

## Phase 0: Research - PENDING

**Status**: Not started

**Research Tasks**:
1. Review Rust `std::io::IsTerminal` trait API and cross-platform behavior
2. Research standard environment variable conventions (NO_COLOR spec, OUTPUT_FORMAT patterns)
3. Evaluate optional `terminal_size` crate for dimension detection
4. Review existing `cli_output` module for integration points
5. Research Windows vs Unix TTY detection differences

**Key Decisions Needed**:
- Whether to use `terminal_size` crate or implement basic dimension detection
- Integration strategy with existing `cli_output::should_use_color()` function
- Cross-platform testing strategy

## Phase 1: Design - PENDING

**Status**: Not started

**Design Artifacts to Generate**:
- [data-model.md](./data-model.md) - Entity definitions (OutputFormat enum, detection function signatures)
- [contracts/api.md](./contracts/api.md) - Function signatures and type contracts
- [quickstart.md](./quickstart.md) - Developer usage guide with examples

**Design Decisions Needed**:
- Function naming conventions (e.g., `is_tty()`, `should_color_output()`, `get_output_format()`)
- Return types (bool, Option<T>, Result<T>)
- Module organization (single file vs submodules)
- Integration with `cli_output` module

