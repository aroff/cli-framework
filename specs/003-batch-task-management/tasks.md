# Implementation Tasks: Batch Task Management

**Feature**: Batch Task Management  
**Branch**: `003-batch-task-management`  
**Date**: 2025-01-27  
**Spec**: [spec.md](./spec.md)  
**Plan**: [plan.md](./plan.md)

## Summary

This document breaks down the batch task management feature into actionable, dependency-ordered tasks organized by user story. Each task is independently implementable and testable.

**Total Tasks**: 53  
**Setup**: 4 tasks  
**Foundational**: 6 tasks  
**User Story 1 (P1)**: 11 tasks  
**User Story 2 (P1)**: 11 tasks  
**User Story 3 (P2)**: 10 tasks  
**Polish**: 11 tasks

## Dependencies

```
Phase 1: Setup
  └─> Phase 2: Foundational (core types)
      └─> Phase 3: User Story 1 (basic batch operations)
          └─> Phase 4: User Story 2 (concurrency control)
              └─> Phase 5: User Story 3 (wait for all, cancellation)
                  └─> Phase 6: Polish
```

**Story Dependencies**:
- User Story 1 (P1) - Parallel File Processing: Independent, can be implemented first
- User Story 2 (P1) - Batch API Operations: Depends on User Story 1 (needs basic batch + concurrency)
- User Story 3 (P2) - Concurrent Data Processing: Depends on User Story 2 (needs wait_for_all)

## Implementation Strategy

**MVP Scope**: User Story 1 (Parallel File Processing) - Provides core batch spawning and result aggregation.

**Incremental Delivery**:
1. **MVP**: Basic batch spawning with result aggregation (User Story 1)
2. **Increment 1**: Add concurrency control (User Story 2)
3. **Increment 2**: Add wait_for_all and cancellation (User Story 3)
4. **Final**: Polish, edge cases, documentation

## Phase 1: Setup

**Goal**: Prepare project structure and dependencies for batch task management.

**Independent Test**: Project compiles and existing tests pass.

### Tasks

- [x] T001 Create test directory structure for batch operations in tests/app/background_tasks.rs
- [x] T002 Create integration test directory structure in tests/integration/batch_operations.rs
- [x] T003 Verify tokio dependencies are available (tokio 1.0 with full features, tokio-util 0.7 with time feature) in Cargo.toml
- [x] T004 Review existing BackgroundTaskManager structure in src/app/background_tasks.rs to understand current implementation

## Phase 2: Foundational

**Goal**: Implement core types and infrastructure needed by all user stories.

**Independent Test**: Types compile and can be instantiated with test data.

### Tasks

- [x] T005 Define TaskIdentifier enum (Provided(String), Index(usize)) in src/app/background_tasks.rs
- [x] T006 Define TaskStatus enum (Success, Failure, Cancelled) in src/app/background_tasks.rs
- [x] T007 Define TaskResult struct with identifier, status, value, error fields in src/app/background_tasks.rs
- [x] T008 Define BatchResult struct with aggregated statistics and results collection in src/app/background_tasks.rs
- [x] T009 Implement TaskIdentifier::display() method returning String representation in src/app/background_tasks.rs
- [x] T010 Implement BatchResult helper methods (all_succeeded, has_failures, has_cancellations, errors, results) in src/app/background_tasks.rs

## Phase 3: User Story 1 - Parallel File Processing (P1)

**Goal**: Enable developers to spawn multiple background tasks concurrently and receive aggregated results.

**Independent Test**: Given a collection of 20 files to process, when spawned as a batch with concurrency limit of 5, then only 5 files are processed simultaneously at any time, and all 20 complete with aggregated results.

**Acceptance Criteria**:
- Applications can provide a collection of tasks to execute
- All tasks are spawned concurrently (not sequentially)
- Framework waits for all tasks to complete before returning
- Results from all tasks are collected and returned (both aggregated statistics and individual task results)
- Empty task collections are handled gracefully (return empty results)

### Tasks

- [x] T011 [US1] Define TaskDefinition struct with task closure and optional identifier in src/app/background_tasks.rs
- [x] T012 [US1] Implement task_definition() helper function for creating TaskDefinition instances in src/app/background_tasks.rs
- [x] T013 [US1] Implement basic spawn_batch() method stub that accepts Vec<TaskDefinition> and returns BatchResult in src/app/background_tasks.rs
- [x] T014 [US1] Implement task spawning logic using tokio::task::JoinSet to spawn all tasks concurrently in src/app/background_tasks.rs
- [x] T015 [US1] Implement result collection loop using JoinSet::join_next() to collect results in completion order in src/app/background_tasks.rs
- [x] T016 [US1] Implement result aggregation logic to calculate total, successful, failed counts from collected TaskResults, preserving optional return values for successful tasks in src/app/background_tasks.rs
- [x] T016a [US1] Implement error message formatting that includes task identifier (Provided or Index) in error messages for failed tasks in src/app/background_tasks.rs
- [x] T017 [US1] Implement empty batch handling (return empty BatchResult immediately) in src/app/background_tasks.rs
- [x] T018 [US1] Add unit tests for basic batch spawning with 10 tasks in tests/app/background_tasks.rs
- [x] T019 [US1] Add unit tests for empty batch handling in tests/app/background_tasks.rs
- [x] T020 [US1] Add integration test for parallel file processing scenario (20 files, verify concurrency) in tests/integration/batch_operations.rs

## Phase 4: User Story 2 - Batch API Operations (P1)

**Goal**: Enable developers to control concurrency limits for batch operations to respect rate limits and system resources.

**Independent Test**: Given 50 API operations to execute with a concurrency limit of 10, when the batch is spawned, then only 10 operations run simultaneously, and all 50 complete with aggregated results.

**Acceptance Criteria**:
- Applications can specify a maximum number of concurrent tasks
- If no concurrency limit is specified, framework uses default limit based on CPU cores (cores * 2)
- Framework enforces maximum concurrency limit (100) to prevent resource exhaustion
- When limit is reached, additional tasks wait until a slot becomes available
- Tasks are started as soon as slots become available (not in fixed batches)
- Concurrency limit applies only to task execution, not to result collection
- A concurrency limit of 1 effectively makes tasks run sequentially

### Tasks

- [x] T021 [US2] Implement CPU core detection using std::thread::available_parallelism() with fallback to 4 in src/app/background_tasks.rs
- [x] T022 [US2] Implement default concurrency limit calculation (CPU cores * 2) in src/app/background_tasks.rs
- [x] T023 [US2] Implement maximum concurrency limit enforcement (100) in spawn_batch() method in src/app/background_tasks.rs
- [x] T024 [US2] Implement concurrency control using tokio::sync::Semaphore with effective limit in spawn_batch() method in src/app/background_tasks.rs
- [x] T025 [US2] Update task spawning logic to acquire semaphore permit before spawning each task in src/app/background_tasks.rs
- [x] T026 [US2] Ensure permits are held during task execution and released when task completes in src/app/background_tasks.rs
- [x] T027 [US2] Add unit tests for concurrency limiting (20 tasks with limit of 5, verify only 5 run simultaneously) in tests/app/background_tasks.rs
- [x] T028 [US2] Add unit tests for default limit calculation (CPU-based) in tests/app/background_tasks.rs
- [x] T029 [US2] Add unit tests for maximum limit enforcement (request 200, verify capped at 100) in tests/app/background_tasks.rs
- [x] T030 [US2] Add unit tests for sequential execution (limit of 1) in tests/app/background_tasks.rs
- [x] T031 [US2] Add integration test for batch API operations scenario (50 operations, limit 10) in tests/integration/batch_operations.rs

## Phase 5: User Story 3 - Concurrent Data Processing (P2)

**Goal**: Enable developers to wait for all active tasks and cancel batch operations while allowing other tasks to continue.

**Independent Test**: Given a dataset split into 100 chunks with a concurrency limit of 20, when the batch is executed, then only 20 chunks are processed simultaneously, and all 100 complete with aggregated results.

**Acceptance Criteria**:
- Applications can wait for all active tasks regardless of how they were spawned
- Results are returned in completion order (not spawn order)
- Wait operation blocks until all tasks finish
- If no tasks are active, operation returns immediately with empty results
- If a task in a batch is cancelled, other tasks in the batch continue executing
- Cancellation of one task does not affect unrelated tasks
- Cancelled tasks are tracked separately in result aggregation (cancelled count)
- Applications can cancel all tasks in a batch if needed
- Individual task results indicate cancelled status for cancelled tasks

### Tasks

- [x] T032 [US3] Implement wait_for_all() method that collects results from all active tasks (individual and batch) in completion order (not spawn order) in src/app/background_tasks.rs
- [x] T033 [US3] Update result collection to handle cancellation status and mark tasks as Cancelled in src/app/background_tasks.rs
- [x] T034 [US3] Implement cancelled count tracking in BatchResult aggregation logic in src/app/background_tasks.rs
- [x] T035 [US3] Update success rate calculation to exclude cancelled tasks (successful / (successful + failed)) in src/app/background_tasks.rs
- [x] T036 [US3] Implement cancel_batch() method that cancels all tasks associated with a batch token in src/app/background_tasks.rs
- [x] T037 [US3] Ensure individual task cancellation doesn't affect other tasks in the batch in src/app/background_tasks.rs
- [x] T038 [US3] Add unit tests for wait_for_all() with mixed individual and batch tasks in tests/app/background_tasks.rs
- [x] T039 [US3] Add unit tests for cancellation handling (cancel batch mid-execution, verify other tasks continue) in tests/app/background_tasks.rs
- [x] T040 [US3] Add unit tests for cancelled task accounting (verify cancelled count, excluded from success rate) in tests/app/background_tasks.rs
- [x] T041 [US3] Add integration test for concurrent data processing scenario (100 chunks, limit 20) in tests/integration/batch_operations.rs

## Phase 6: Polish & Cross-Cutting Concerns

**Goal**: Complete feature with error handling, edge cases, documentation, and backward compatibility verification.

**Independent Test**: All edge cases handled gracefully, backward compatibility maintained, documentation complete.

### Tasks

- [ ] T042 Implement timeout handling (tasks exceeding timeout treated as failures) in src/app/background_tasks.rs
  Note: Timeout errors are handled via existing error preservation mechanism (FR-5); this task implements the timeout detection and error creation.
- [ ] T043 Add unit tests for timeout handling (verify timeout errors included in failed count) in tests/app/background_tasks.rs
- [ ] T044 Add unit tests for single task batch (verify works correctly) in tests/app/background_tasks.rs
- [ ] T045 Add unit tests for very large batch (1000+ tasks, verify memory and performance) in tests/app/background_tasks.rs
- [ ] T046 Add unit tests for mixed success/failure scenarios (verify aggregation correct) in tests/app/background_tasks.rs
- [ ] T047 Add unit tests for all tasks fail scenario (verify 0 successes, all failures) in tests/app/background_tasks.rs
- [ ] T048 Verify backward compatibility (existing spawn, spawn_streaming, spawn_periodic methods unchanged) in src/app/background_tasks.rs
- [ ] T049 Add backward compatibility tests (verify existing APIs still work) in tests/app/background_tasks.rs
- [x] T050 Update module documentation with batch task management examples in src/app/background_tasks.rs
- [x] T051 Run cargo fmt and cargo clippy to ensure code formatting and linting pass
- [x] T052 Run full test suite to verify all tests pass (cargo test)

## Parallel Execution Opportunities

### Within User Story 1
- T011, T012 can be done in parallel (both are type definitions)
- T018, T019 can be done in parallel (both are test files)

### Within User Story 2
- T021, T022 can be done in parallel (CPU detection and default calculation)
- T027, T028, T029, T030 can be done in parallel (all are test tasks)

### Within User Story 3
- T032, T033 can be done sequentially (wait_for_all depends on cancellation handling)
- T038, T039, T040 can be done in parallel (all are test tasks)

### Cross-Phase
- Test tasks (T018-T020, T027-T031, T038-T041) can be done in parallel with implementation tasks from their respective phases

## Notes

- All tasks must maintain backward compatibility with existing `BackgroundTaskManager` API
- Task identifiers are optional; positional index (0-based) used as fallback
- Results are collected in completion order, not spawn order
- Maximum concurrency limit of 100 is enforced regardless of user request
- Default limit is CPU cores * 2, with fallback to 4 if detection fails
- Cancelled tasks are tracked separately and excluded from success rate calculation
- Timeout errors are treated as failures and included in failed count

