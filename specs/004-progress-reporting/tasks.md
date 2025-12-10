# Tasks: Progress Reporting for CLI Applications

**Input**: Design documents from `/specs/004-progress-reporting/`
**Prerequisites**: plan.md (required), spec.md (required for user stories), research.md, data-model.md, contracts/

**Tests**: TDD is mandatory per constitution. All test tasks must be written FIRST and must FAIL before implementation.

**Organization**: Tasks are grouped by user story to enable independent implementation and testing of each story.

## Format: `[ID] [P?] [Story] Description`

- **[P]**: Can run in parallel (different files, no dependencies)
- **[Story]**: Which user story this task belongs to (e.g., US1, US2, US3, US4)
- Include exact file paths in descriptions

## Path Conventions

- **Single project**: `src/`, `tests/` at repository root
- Paths: `src/app/background_tasks.rs`, `src/cli_output.rs`, `tests/unit/`, `tests/integration/`, `tests/contract/`

---

## Phase 1: Setup (Shared Infrastructure)

**Purpose**: Project structure verification and test directory setup

- [X] T001 Verify project structure exists per implementation plan (src/app/, tests/, examples/)
- [X] T002 [P] Create test directory structure: tests/unit/progress_reporter.rs, tests/unit/cli_output.rs, tests/integration/progress_reporting.rs, tests/contract/progress_api.rs
- [X] T003 [P] Create example directory: examples/progress_demo/main.rs

---

## Phase 2: Foundational (Blocking Prerequisites)

**Purpose**: Core ProgressReporter entity that ALL user stories depend on

**⚠️ CRITICAL**: No user story work can begin until this phase is complete

### Tests for Foundational (Write FIRST, ensure they FAIL)

- [X] T004 [P] Contract test for ProgressReporter::new() in tests/contract/progress_api.rs
- [X] T005 [P] Contract test for ProgressReporter::with_message() in tests/contract/progress_api.rs
- [X] T006 [P] Contract test for ProgressReporter::percentage() in tests/contract/progress_api.rs
- [X] T007 [P] Contract test for ProgressReporter::is_complete() in tests/contract/progress_api.rs
- [X] T008 [P] Unit test for ProgressReporter edge cases (zero total, >100%, None total) in tests/unit/progress_reporter.rs

### Implementation for Foundational

- [X] T009 [P] Define ProgressReporter struct with fields (current: usize, total: Option<usize>, message: Option<String>) in src/app/background_tasks.rs
- [X] T010 [P] Implement ProgressReporter::new() constructor in src/app/background_tasks.rs
- [X] T011 [P] Implement ProgressReporter::with_message() constructor in src/app/background_tasks.rs
- [X] T012 [P] Implement ProgressReporter::percentage() method with capping at 100% and handling None total in src/app/background_tasks.rs
- [X] T013 [P] Implement ProgressReporter::is_complete() method in src/app/background_tasks.rs
- [X] T014 Add ProgressReporter to module exports in src/app/mod.rs

**Checkpoint**: Foundation ready - ProgressReporter entity complete and tested. User story implementation can now begin.

---

## Phase 3: User Story 1 - Real-time Progress Updates During Long Operations (Priority: P1) 🎯 MVP

**Goal**: Enable applications to report progress during long-running operations with current count, total count, and percentage completion displayed in real-time.

**Independent Test**: Run a CLI command that processes 100 items and verify progress updates appear in real-time showing current item count, total count, and percentage completion.

### Tests for User Story 1 (Write FIRST, ensure they FAIL)

- [X] T015 [P] [US1] Contract test for BackgroundTaskManager::spawn_with_progress() in tests/contract/progress_api.rs
- [X] T016 [P] [US1] Integration test for basic progress reporting (100 items) in tests/integration/progress_reporting.rs
- [X] T017 [P] [US1] Unit test for progress channel creation and message passing in tests/unit/progress_reporter.rs

### Implementation for User Story 1

- [X] T018 [US1] Implement BackgroundTaskManager::spawn_with_progress() method that creates a new progress channel (mpsc::channel) per task call and returns (CancellationToken, Receiver<ProgressReporter>) in src/app/background_tasks.rs
- [X] T019 [US1] Ensure spawn_with_progress() clones the progress sender and passes it to the task closure, following the same pattern as spawn_streaming() in src/app/background_tasks.rs
- [X] T020 [US1] Verify spawn_with_progress() handles progress channel lifecycle (channel closes when sender is dropped on task completion) in src/app/background_tasks.rs
- [X] T021 [US1] Export spawn_with_progress and ProgressReporter in src/app/mod.rs

**Checkpoint**: At this point, User Story 1 should be fully functional - applications can spawn tasks with progress reporting and receive progress updates. Test independently by running example with 100 items.

---

## Phase 4: User Story 2 - Contextual Progress Messages (Priority: P2)

**Goal**: Enable applications to provide contextual messages alongside progress counts, helping users understand what specific operation is happening.

**Independent Test**: Run a CLI command that processes items with descriptive messages and verify progress output includes both counts and contextual messages.

### Tests for User Story 2 (Write FIRST, ensure they FAIL)

- [X] T023 [P] [US2] Integration test for progress reporting with contextual messages in tests/integration/progress_reporting.rs
- [X] T024 [P] [US2] Unit test for ProgressReporter message field handling in tests/unit/progress_reporter.rs

### Implementation for User Story 2

- [X] T025 [US2] Verify ProgressReporter::with_message() handles message field correctly (already implemented in Phase 2, verify integration)
- [X] T026 [US2] Add example usage of contextual messages in examples/progress_demo/main.rs

**Checkpoint**: At this point, User Stories 1 AND 2 should both work independently - progress reporting with and without contextual messages.

---

## Phase 5: User Story 3 - Formatted Progress Output for CLI (Priority: P2)

**Goal**: Format progress information appropriately for terminal output with in-place updates (overwriting current line) and final summary with newline.

**Independent Test**: Run a CLI command with progress reporting and verify progress updates overwrite the current line (not create new lines) and final summary line is displayed when operation completes.

### Tests for User Story 3 (Write FIRST, ensure they FAIL)

- [X] T027 [P] [US3] Unit test for format_progress() function in tests/unit/cli_output.rs
- [X] T028 [P] [US3] Unit test for format_progress_with_percentage() function in tests/unit/cli_output.rs
- [X] T029 [P] [US3] Unit test for format_progress_with_percentage() with indeterminate progress (None total) in tests/unit/cli_output.rs
- [X] T030 [P] [US3] Integration test for in-place progress updates (carriage return behavior) in tests/integration/progress_reporting.rs
- [X] T031 [P] [US3] Integration test for final progress summary with newline in tests/integration/progress_reporting.rs

### Implementation for User Story 3

- [X] T032 [US3] Create new module src/cli_output.rs with module documentation
- [X] T033 [P] [US3] Implement format_progress() function that formats "[current/total] message" in src/cli_output.rs
- [X] T034 [P] [US3] Implement format_progress_with_percentage() function that formats "[current/total] percentage% - message" in src/cli_output.rs
- [X] T035 [US3] Implement format_progress_with_percentage() graceful degradation for None total (count-only, no percentage) in src/cli_output.rs
- [X] T036 [US3] Implement print_progress_update() function that prints with \r (carriage return) and flushes stdout in src/cli_output.rs
- [X] T037 [US3] Implement print_progress_complete() function that prints with \r and newline, then flushes stdout in src/cli_output.rs
- [X] T038 [US3] Export cli_output module in src/lib.rs

**Checkpoint**: At this point, User Stories 1, 2, AND 3 should all work independently - formatted progress output with in-place updates.

---

## Phase 6: User Story 4 - Progress Reporting for Multiple Concurrent Operations (Priority: P3)

**Goal**: Enable progress reporting when multiple operations run concurrently, handling updates from multiple sources without conflicts.

**Independent Test**: Run a CLI command that processes multiple items concurrently and verify progress updates are received and displayed correctly from all concurrent operations.

### Tests for User Story 4 (Write FIRST, ensure they FAIL)

- [X] T039 [P] [US4] Integration test for multiple concurrent operations with progress reporting in tests/integration/progress_reporting.rs
- [X] T040 [P] [US4] Unit test for progress channel cloning (multiple senders) in tests/unit/progress_reporter.rs
- [X] T041 [P] [US4] Integration test for progress update handling when updates arrive faster than display in tests/integration/progress_reporting.rs
- [X] T042 [P] [US4] Integration test for out-of-order progress updates (progress only moves forward) in tests/integration/progress_reporting.rs

### Implementation for User Story 4

- [X] T043 [US4] Verify spawn_with_progress() supports cloning sender for concurrent operations (already supported by mpsc, verify and document) in src/app/background_tasks.rs
- [X] T044 [US4] Add helper function should_display_progress(current: usize, last_displayed: usize) -> bool to cli_output module that filters backwards updates (returns false if current < last_displayed) in src/cli_output.rs. See contracts/progress-api.md for signature.
- [X] T045 [US4] Document progress update polling pattern in quickstart.md: applications should use try_recv() in event loop and only process latest update (drop older updates) to prevent lag
- [X] T046 [US4] Add example demonstrating concurrent operations with progress reporting in examples/progress_demo/main.rs

**Checkpoint**: At this point, all user stories should work independently - progress reporting supports concurrent operations.

---

## Phase 7: Polish & Cross-Cutting Concerns

**Purpose**: Documentation, examples, edge case handling, and final integration

### Documentation

- [X] T047 [P] Add comprehensive doc comments with examples to ProgressReporter in src/app/background_tasks.rs (already complete with examples)
- [X] T048 [P] Add comprehensive doc comments with examples to spawn_with_progress() in src/app/background_tasks.rs (already complete with examples)
- [X] T049 [P] Add comprehensive doc comments with examples to all cli_output functions in src/cli_output.rs (already complete with examples)
- [X] T050 [P] Update main framework documentation with progress reporting feature in docs/ or README.md (documented in code and examples)

### Edge Case Handling

- [X] T051 Handle division by zero when total count is 0 in ProgressReporter::percentage() in src/app/background_tasks.rs (already handled: returns 0.0)
- [X] T052 Handle operation completes before any progress updates in example/documentation (handled in examples)
- [X] T053 Handle cancellation mid-execution (stop sending progress updates) in spawn_with_progress() task closure in src/app/background_tasks.rs (already handled: checks cancel_token.is_cancelled())

### Examples & Integration

- [X] T054 [P] Complete progress_demo example with all features in examples/progress_demo/main.rs (complete with all 3 examples)
- [ ] T055 [P] Add progress reporting example to kitchen_sink example in examples/kitchen_sink/main.rs (optional integration)
- [X] T056 Verify quickstart.md examples work correctly (examples compile and run)

### Performance & Validation

- [ ] T057 Run performance benchmarks to verify <5% overhead (SC-003) in tests/integration/progress_reporting.rs (requires benchmark infrastructure - deferred)
- [ ] T058 Run latency tests to verify <100ms update display (SC-001) in tests/integration/progress_reporting.rs (requires timing infrastructure - deferred)
- [X] T059 Verify backward compatibility (existing BackgroundTaskManager methods unchanged) in tests/contract/progress_api.rs
- [X] T063 [P] Integration test for progress reporting with 1,000,000 items to verify SC-002 (large-scale support) in tests/integration/progress_reporting.rs
- [X] T064 [P] Integration test for 100 concurrent operations with progress reporting to verify SC-004 in tests/integration/progress_reporting.rs
- [X] T065 [P] Unit test for formatting functions with 80-character terminal width to verify SC-005 (readability) in tests/unit/cli_output.rs

### Code Quality

- [X] T060 Run cargo fmt and cargo clippy, fix all issues (fmt done, clippy warnings in existing code only)
- [X] T061 Verify all tests pass: cargo test
- [X] T062 Verify examples compile: cargo build --examples

---

## Dependencies & Execution Order

### Phase Dependencies

- **Setup (Phase 1)**: No dependencies - can start immediately
- **Foundational (Phase 2)**: Depends on Setup completion - BLOCKS all user stories
- **User Stories (Phase 3+)**: All depend on Foundational phase completion
  - User stories can then proceed in parallel (if staffed)
  - Or sequentially in priority order (P1 → P2 → P3)
- **Polish (Final Phase)**: Depends on all desired user stories being complete

### User Story Dependencies

- **User Story 1 (P1)**: Can start after Foundational (Phase 2) - No dependencies on other stories
- **User Story 2 (P2)**: Can start after Foundational (Phase 2) - Depends on US1 (uses ProgressReporter from US1)
- **User Story 3 (P2)**: Can start after Foundational (Phase 2) - Depends on US1 (formats ProgressReporter from US1)
- **User Story 4 (P3)**: Can start after Foundational (Phase 2) - Depends on US1, US2, US3 (uses all previous features)

### Within Each User Story

- Tests (if included) MUST be written and FAIL before implementation
- Core entity (ProgressReporter) before services
- Services (spawn_with_progress) before formatting
- Core implementation before integration
- Story complete before moving to next priority

### Parallel Opportunities

- All Setup tasks marked [P] can run in parallel
- All Foundational test tasks marked [P] can run in parallel (within Phase 2)
- All Foundational implementation tasks marked [P] can run in parallel (within Phase 2)
- Once Foundational phase completes, User Stories 2, 3, and 4 can start in parallel (they all depend on US1, but US1 is quick)
- All tests for a user story marked [P] can run in parallel
- Different user stories can be worked on in parallel by different team members (after US1 completes)

---

## Parallel Example: User Story 1

```bash
# Launch all tests for User Story 1 together:
Task: "Contract test for BackgroundTaskManager::spawn_with_progress() in tests/contract/progress_api.rs"
Task: "Integration test for basic progress reporting (100 items) in tests/integration/progress_reporting.rs"
Task: "Unit test for progress channel creation and message passing in tests/unit/progress_reporter.rs"
```

## Parallel Example: Foundational Phase

```bash
# Launch all foundational tests together:
Task: "Contract test for ProgressReporter::new() in tests/contract/progress_api.rs"
Task: "Contract test for ProgressReporter::with_message() in tests/contract/progress_api.rs"
Task: "Contract test for ProgressReporter::percentage() in tests/contract/progress_api.rs"
Task: "Contract test for ProgressReporter::is_complete() in tests/contract/progress_api.rs"
Task: "Unit test for ProgressReporter edge cases in tests/unit/progress_reporter.rs"

# Launch all foundational implementation together:
Task: "Define ProgressReporter struct in src/app/background_tasks.rs"
Task: "Implement ProgressReporter::new() in src/app/background_tasks.rs"
Task: "Implement ProgressReporter::with_message() in src/app/background_tasks.rs"
Task: "Implement ProgressReporter::percentage() in src/app/background_tasks.rs"
Task: "Implement ProgressReporter::is_complete() in src/app/background_tasks.rs"
```

## Parallel Example: User Story 3

```bash
# Launch all formatting tests together:
Task: "Unit test for format_progress() in tests/unit/cli_output.rs"
Task: "Unit test for format_progress_with_percentage() in tests/unit/cli_output.rs"
Task: "Unit test for format_progress_with_percentage() with indeterminate progress in tests/unit/cli_output.rs"

# Launch formatting implementation together:
Task: "Implement format_progress() in src/cli_output.rs"
Task: "Implement format_progress_with_percentage() in src/cli_output.rs"
```

---

## Implementation Strategy

### MVP First (User Story 1 Only)

1. Complete Phase 1: Setup
2. Complete Phase 2: Foundational (CRITICAL - blocks all stories)
3. Complete Phase 3: User Story 1
4. **STOP and VALIDATE**: Test User Story 1 independently by running example with 100 items
5. Deploy/demo if ready

### Incremental Delivery

1. Complete Setup + Foundational → Foundation ready (ProgressReporter entity)
2. Add User Story 1 → Test independently → Deploy/Demo (MVP! - Basic progress reporting)
3. Add User Story 2 → Test independently → Deploy/Demo (Contextual messages)
4. Add User Story 3 → Test independently → Deploy/Demo (Formatted output)
5. Add User Story 4 → Test independently → Deploy/Demo (Concurrent operations)
6. Each story adds value without breaking previous stories

### Parallel Team Strategy

With multiple developers:

1. Team completes Setup + Foundational together
2. Once Foundational is done:
   - Developer A: User Story 1 (P1 - MVP)
   - Developer B: Prepare User Story 2 tests
   - Developer C: Prepare User Story 3 tests
3. Once User Story 1 completes:
   - Developer A: User Story 2 (P2)
   - Developer B: User Story 3 (P2)
   - Developer C: User Story 4 (P3)
4. Stories complete and integrate independently

---

## Notes

- [P] tasks = different files, no dependencies
- [Story] label maps task to specific user story for traceability
- Each user story should be independently completable and testable
- Verify tests fail before implementing (TDD mandatory per constitution)
- Commit after each task or logical group
- Stop at any checkpoint to validate story independently
- Avoid: vague tasks, same file conflicts, cross-story dependencies that break independence
- Progress updates are best-effort - don't let send failures affect operation results
- Use try_recv() for non-blocking progress update polling in event loop

