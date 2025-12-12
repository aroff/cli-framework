# Implementation Plan: Task Result Aggregation

**Branch**: `007-task-result-aggregation` | **Date**: 2025-01-27 | **Spec**: [spec.md](./spec.md)
**Input**: Feature specification from `/specs/007-task-result-aggregation/spec.md`

**Note**: This template is filled in by the `/speckit.plan` command. See `.specify/templates/commands/plan.md` for the execution workflow.

## Summary

Provide utilities for aggregating and summarizing results from batch task operations, enabling CLI applications to easily report statistics (success/failure counts, error summaries) and generate user-friendly reports without manual aggregation logic. This feature builds on the existing `BatchResult` type from batch task management (003) and adds convenience methods for formatting, filtering, merging, and summarizing results.

## Technical Context

**Language/Version**: Rust 1.75+ (edition 2021)  
**Primary Dependencies**: 
- `anyhow` (1.0) - for error types (already in dependencies)
- No new external dependencies required

**Storage**: N/A (in-memory aggregation utilities)  
**Testing**: cargo test (Rust standard testing framework)  
**Target Platform**: Cross-platform (Linux, macOS, Windows) - CLI framework  
**Project Type**: Library (Rust crate) - extension module  
**Performance Goals**: 
- Summary message generation completes in under 10ms for batches up to 10,000 tasks (SC-005)
- Aggregation correctly calculates statistics for batches ranging from 1 to 1,000,000 tasks (SC-002)
- Success rate calculations accurate to within 0.1% (SC-003)

**Constraints**: 
- Must work seamlessly with existing `BatchResult` from batch task management
- Must maintain backward compatibility (all new functionality is additive)
- Must support opt-in error collection limits for very large batches (100,000+ tasks)
- Must preserve task identification in error formatting

**Scale/Scope**: 
- Support batches from 1 to 1,000,000 tasks
- Support merging up to 100 separate batch operations (SC-007)
- Error collection: unlimited by default, opt-in limits for memory efficiency
- Library feature extension; affects `src/app/background_tasks.rs` or new aggregation module

## Constitution Check

*GATE: Must pass before Phase 0 research. Re-check after Phase 1 design.*

### I. Library-First ✅
- **Status**: PASS
- **Rationale**: Aggregation utilities are self-contained within the library crate. Applications opt-in by using the utilities. No hosting code required.

### II. Test-First (NON-NEGOTIABLE) ✅
- **Status**: PASS
- **Rationale**: All aggregation functions will have unit tests written first. Test cases cover all functional requirements, edge cases, and success criteria.

### III. Documentation & API Design ✅
- **Status**: PASS
- **Rationale**: All public functions will have doc comments with examples. The aggregation utilities will be documented in the library documentation.

### IV. Observability ✅
- **Status**: PASS
- **Rationale**: Aggregation functions use `anyhow::Result` for error handling where appropriate. No additional observability requirements for this feature.

### V. Simplicity & Error Handling ✅
- **Status**: PASS
- **Rationale**: Straightforward aggregation and formatting functions. Uses existing `anyhow::Error` for errors. No complex abstractions needed.

**Gate Result**: ✅ **PASS** - All constitution principles satisfied.

## Project Structure

### Documentation (this feature)

```text
specs/007-task-result-aggregation/
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
│   └── background_tasks.rs    # Extended with aggregation utilities
└── lib.rs

tests/
├── app/
│   └── background_tasks.rs    # Tests for aggregation utilities
└── integration/
    └── result_aggregation.rs  # Integration tests
```

**Structure Decision**: Extend existing `BatchResult` in `src/app/background_tasks.rs` with aggregation utility methods and standalone aggregation functions. This maintains consistency with existing batch task management code and provides a cohesive API.

## Complexity Tracking

> **Fill ONLY if Constitution Check has violations that must be justified**

N/A - No violations detected.

## Phase 0: Research - COMPLETE

All technical decisions have been resolved. See [research.md](./research.md) for details.

**Key Decisions**:
- Extend existing `BatchResult` type with convenience methods
- Create standalone aggregation utility functions for flexibility
- Support error filtering with separate filtered errors collection
- Opt-in error collection limits for memory efficiency
- Task identification always included in error formatting

## Phase 1: Design - COMPLETE

Data model, API contracts, and quickstart guide have been created.

**Generated Artifacts**:
- [data-model.md](./data-model.md) - Entity definitions and relationships
- [contracts/api.md](./contracts/api.md) - Function signatures and type contracts
- [quickstart.md](./quickstart.md) - Developer usage guide

**Design Summary**:
- Extends `BatchResult` with convenience methods (summary generation, error formatting)
- Provides standalone aggregation functions for filtering and merging
- Maintains full backward compatibility with existing batch task management
- Supports error filtering with auditability (filtered errors preserved separately)
- Task identification included in all error formatting

## Phase 2: Task Breakdown

**Status**: Pending (run `/speckit.tasks` to generate)

**Next Steps**:
1. Run `/speckit.tasks` to break down implementation into concrete tasks
2. Begin implementation following the task breakdown
3. Write tests alongside implementation (TDD approach)

