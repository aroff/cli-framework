<!--
Sync Impact Report:
Version: 0.0.0 → 1.0.0 (Initial constitution)
Modified principles: N/A (initial creation)
Added sections: Core Principles, Technology Stack, Development Workflow, Governance
Removed sections: N/A
Templates requiring updates:
  ✅ plan-template.md - Constitution Check section references constitution
  ✅ spec-template.md - No direct references, compatible
  ✅ tasks-template.md - No direct references, compatible
Follow-up TODOs: None
-->

# CLI Framework Constitution

## Core Principles

### I. Library-First
The framework MUST be a standalone, independently testable library crate. Every feature MUST be self-contained with clear purpose. The framework MUST minimize hosting code for application developers, allowing them to focus on commands and operations rather than terminal management. Rationale: Enables reuse, independent testing, and clear separation of concerns between framework and application code.

### II. Test-First (NON-NEGOTIABLE)
TDD mandatory: Tests written → User approved → Tests fail → Then implement. Red-Green-Refactor cycle strictly enforced. All core abstractions (View, DataSource, Command) MUST have contract tests. Integration tests MUST verify user stories independently. Unit tests MUST cover all public API methods. Rationale: Ensures correctness, prevents regressions, and maintains confidence during refactoring.

### III. Documentation & API Design
Public API MUST be documented with examples. All public traits, structs, and functions MUST have doc comments explaining purpose, usage, and error conditions. Examples directory MUST demonstrate all major features. Rationale: Enables adoption, reduces support burden, and clarifies intended usage patterns.

### IV. Observability
Framework MUST provide optional debug logging and metrics that applications can enable. Framework MUST be compatible with OpenTelemetry for integration with standard observability tooling. Observability features MUST be opt-in to keep the framework lightweight. Rationale: Enables troubleshooting framework behavior without imposing observability requirements on all applications.

### V. Simplicity & Error Handling
Code MUST be straightforward, clean, and non-convoluted. YAGNI principles apply: build only what is needed now. Framework MUST use `anyhow::Result` for framework operations to provide clear error context. Complex solutions MUST be justified against simpler alternatives. Rationale: Maintains codebase readability, reduces maintenance burden, and enables faster onboarding.

## Technology Stack

**Language**: Rust (latest stable, minimum 1.75+)  
**Primary Dependencies**: 
- `ratatui` (TUI rendering library)
- `crossterm` (terminal I/O and event handling)
- `anyhow` (error handling)
- `serde` (serialization for configuration)
- OpenTelemetry crates (optional observability integration)

**Testing**: `cargo test` with unit tests, integration tests, and contract tests for View/DataSource traits. TUI testing uses `ratatui::backend::TestBackend` for headless testing of rendering logic.

**Target Platform**: Linux, macOS, Windows (terminal-based CLI applications)

**Project Type**: Library crate (`tui-framework`) that applications statically link

## Development Workflow

**Test-Driven Development**: All features MUST follow TDD: write tests first, ensure they fail, then implement. Contract tests MUST be written for all trait implementations.

**Code Review**: All PRs/reviews MUST verify constitution compliance. Complexity MUST be justified. Public API changes MUST include documentation updates.

**Quality Gates**: 
- All tests MUST pass before merge
- Public API MUST be documented
- Examples MUST be updated for new features
- Contract tests MUST pass for trait implementations

**Reference Application**: `examples/kitchen_sink` serves as the primary integration test bench and demonstrates all framework features.

## Governance

Constitution supersedes all other practices. Amendments require documentation, approval, and migration plan. All PRs/reviews MUST verify compliance with constitution principles. Complexity MUST be justified against simpler alternatives. Use `.specify/templates/` for runtime development guidance.

**Version**: 1.0.0 | **Ratified**: 2025-11-17 | **Last Amended**: 2025-11-19
