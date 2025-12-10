# Tasks: Async Runtime Migration

**Input**: Design documents from `/specs/002-async-migration/`
**Prerequisites**: plan.md (required), spec.md (required for user stories), research.md, data-model.md, contracts/

**Tests**: Tests are included per Constitution requirement (Test-First, NON-NEGOTIABLE). All tests must be written first and fail before implementation.

**Organization**: Tasks are grouped by user story to enable independent implementation and testing of each story.

## Format: `[ID] [P?] [Story] Description`

- **[P]**: Can run in parallel (different files, no dependencies)
- **[Story]**: Which user story this task belongs to (e.g., US1, US2, US3)
- Include exact file paths in descriptions

## Path Conventions

- **Single project**: `src/`, `tests/` at repository root
- Paths shown below assume single project structure per plan.md

## Phase 1: Setup (Shared Infrastructure)

**Purpose**: Project initialization and dependency setup

- [X] T001 Update Cargo.toml to add tokio, async-trait, tokio-util dependencies in Cargo.toml
- [X] T002 [P] Update version to 0.2.0 in Cargo.toml (breaking change)
- [X] T003 [P] Update lib.rs documentation to mention async runtime and Tokio requirement in src/lib.rs

---

## Phase 2: Foundational (Blocking Prerequisites)

**Purpose**: Core async infrastructure that MUST be complete before ANY user story can be implemented

**⚠️ CRITICAL**: No user story work can begin until this phase is complete

- [X] T004 Add Send + Sync bounds to AppContext trait in src/app/context.rs
- [X] T005 Create BackgroundTaskManager struct in src/app/background_tasks.rs
- [X] T006 [P] Implement background task spawning and result handling in src/app/background_tasks.rs
- [X] T007 [P] Implement cancellation token support in src/app/background_tasks.rs
- [X] T008 Convert Runtime to support async operations (add Tokio runtime handle) in src/app/runtime.rs
- [X] T009 Implement async terminal event reading using spawn_blocking in src/app/runtime.rs
- [X] T010 Add loading indicator tracking to Runtime in src/app/runtime.rs
- [X] T011 Convert App::run() to async function in src/app/builder.rs
- [X] T012 Implement async event loop with tokio::select! in src/app/builder.rs
- [X] T013 Add Tokio runtime initialization in AppBuilder::build() in src/app/builder.rs

**Checkpoint**: Foundation ready - async runtime operational, event loop async, background tasks supported. User story implementation can now begin.

---

## Phase 3: User Story 1 - Build TUI with Async Service Integration (Priority: P1) 🎯 MVP

**Goal**: Enable direct integration with async services (e.g., FastSkill, HTTP APIs, databases) without blocking the UI during network operations. Developers can use async service clients directly in DataSource and Command implementations.

**Independent Test**: Given a TUI application using an async HTTP client (e.g., reqwest), when the application fetches data from a remote API during view refresh, then the UI remains responsive and interactive during the network request, and data appears when the request completes.

### Tests for User Story 1 ⚠️

> **NOTE: Write these tests FIRST, ensure they FAIL before implementation**

- [X] T014 [P] [US1] Contract test for async DataSource::refresh in tests/contract/test_async_data_source.rs
- [X] T015 [P] [US1] Contract test for async View::handle_event in tests/contract/test_async_view.rs
- [X] T016 [P] [US1] Contract test for async Command execution in tests/contract/test_async_command.rs
- [X] T017 [US1] Integration test: async DataSource refresh doesn't block UI in tests/integration/test_async_integration.rs
- [X] T018 [US1] Integration test: async command execution doesn't block UI in tests/integration/test_async_integration.rs
- [X] T019 [US1] Unit test: concurrent DataSource refresh operations in tests/unit/test_concurrent_refresh.rs

### Implementation for User Story 1

- [X] T020 [P] [US1] Convert DataSource trait to async using async-trait in src/data_source/data_source_trait.rs
- [X] T021 [P] [US1] Convert View trait handle_event to async using async-trait in src/view/view_trait.rs
- [X] T022 [P] [US1] Convert Command execute function to async (return Pin<Box<dyn Future>>) in src/command/mod.rs
- [X] T023 [US1] Update AppBuilder to handle async trait registrations in src/app/builder.rs
- [X] T024 [US1] Implement async DataSource refresh call in event loop in src/app/builder.rs
- [X] T025 [US1] Implement async View handle_event call in event loop in src/app/builder.rs
- [X] T026 [US1] Implement async Command execution in event loop in src/app/builder.rs
- [X] T027 [US1] Update GridView to work with async DataSource in src/widget/grid.rs
- [X] T028 [US1] Add error handling for async operations in src/app/builder.rs

**Checkpoint**: At this point, User Story 1 should be fully functional. Applications can use async service clients directly in DataSource and Command implementations, and the UI remains responsive during async operations.

---

## Phase 4: User Story 2 - Responsive UI During Long Operations (Priority: P1)

**Goal**: UI remains interactive and responsive during long-running operations (e.g., fetching large datasets, running database queries), allowing users to continue navigating, viewing other data, or canceling operations.

**Independent Test**: Given a TUI application performing a long-running data fetch (simulated 5-second delay), when the fetch is triggered, then the user can still interact with the UI (navigate views, open command palette, view help) during the operation, and the UI updates when the operation completes.

### Tests for User Story 2 ⚠️

> **NOTE: Write these tests FIRST, ensure they FAIL before implementation**

- [X] T029 [P] [US2] Integration test: UI remains responsive during 5-second async operation in tests/test_ui_responsiveness.rs
- [X] T030 [P] [US2] Integration test: user can navigate views during async operation in tests/test_ui_responsiveness.rs
- [X] T031 [P] [US2] Integration test: user can open command palette during async operation in tests/test_ui_responsiveness.rs
- [X] T032 [US2] Unit test: concurrent operations don't block each other in tests/test_concurrent_operations.rs
- [X] T033 [US2] Performance test: user interactions respond within 50ms during async operations in tests/test_performance.rs

### Implementation for User Story 2

- [X] T034 [US2] Implement operation cancellation on view switch in src/app/builder.rs
- [X] T035 [US2] Add cancellation token support to async operations in src/app/builder.rs
- [X] T036 [US2] Implement configurable timeout system (default 30s) in src/app/runtime.rs
- [X] T037 [US2] Add timeout handling to async operations in src/app/builder.rs
- [X] T038 [US2] Ensure event loop processes user input during async operations in src/app/builder.rs
- [X] T039 [US2] Verify render loop continues during async operations in src/app/builder.rs
- [X] T040 [US2] Add concurrent operation support using tokio::join! in src/app/builder.rs

**Checkpoint**: At this point, User Stories 1 AND 2 should both work independently. UI remains responsive during long operations, users can interact with the UI, and operations can be cancelled or timeout.

---

## Phase 5: User Story 3 - Streaming Logs and Real-time Updates (Priority: P2)

**Goal**: Framework supports background tasks that continuously update the UI with new data (streaming logs, real-time updates) without blocking the event loop.

**Independent Test**: Given a LogView connected to a streaming log source, when logs are generated continuously, then new log lines appear in the UI in real-time without blocking user interactions or other operations.

### Tests for User Story 3 ⚠️

> **NOTE: Write these tests FIRST, ensure they FAIL before implementation**

- [X] T041 [P] [US3] Integration test: streaming logs appear in real-time in tests/test_streaming.rs
- [X] T042 [P] [US3] Integration test: streaming doesn't block user interactions in tests/test_streaming.rs
- [X] T043 [P] [US3] Integration test: multiple streaming sources work concurrently in tests/test_streaming.rs
- [X] T044 [US3] Unit test: background task results appear in UI within 100ms in tests/test_streaming.rs
- [X] T045 [US3] Performance test: streaming updates appear within 100ms latency in tests/test_performance.rs

### Implementation for User Story 3

- [X] T046 [US3] Enhance BackgroundTaskManager for streaming data support in src/app/background_tasks.rs
- [X] T047 [US3] Implement periodic background task execution in src/app/background_tasks.rs
- [X] T048 [US3] Update LogView to consume streaming data from background tasks in src/widget/log.rs
- [X] T049 [US3] Add channel-based communication for streaming updates in src/app/background_tasks.rs
- [X] T050 [US3] Integrate background task results into main event loop in src/app/builder.rs
- [X] T051 [US3] Update LogSource trait for async streaming if needed in src/data_source/log.rs
- [X] T052 [US3] Ensure streaming updates don't conflict with user interactions in src/app/builder.rs

**Checkpoint**: At this point, all user stories should be independently functional. Framework supports streaming data, background tasks, and real-time updates without blocking the UI.

---

## Phase 6: Loading Indicators & Error Handling

**Purpose**: Automatic loading indicators and graceful error handling (cross-cutting, supports all user stories)

- [X] T053 [P] Implement loading indicator tracking system in src/app/runtime.rs
- [X] T054 [P] Add loading indicator display in status bar or view area in src/widget/status_bar.rs
- [X] T055 [P] Show loading indicator within 100ms of operation start in src/app/builder.rs
- [X] T056 [P] Hide loading indicator when operation completes in src/app/builder.rs
- [X] T057 Implement async error handling and display in src/app/builder.rs
- [X] T058 Add error messages to status bar for async operation failures in src/app/builder.rs
- [X] T059 Add error details to modal for async operation failures in src/widget/modal.rs
- [X] T060 Ensure 100% of async errors show clear, actionable messages in src/app/builder.rs
- [X] T081 [P] Assess and update RetryExecutor for async compatibility in src/retry/executor_async.rs

---

## Phase 7: Examples & Documentation

**Purpose**: Update all examples and create migration documentation

- [X] T061 [P] Update simple example to use async patterns in examples/simple/main.rs
- [X] T062 [P] Update multi_view example to use async patterns in examples/multi_view/main.rs
- [X] T063 [P] Update kitchen_sink example to use async patterns in examples/kitchen_sink/main.rs
- [ ] T064 Verify all examples migrate with <20 lines of changes per example
- [ ] T065 Create async migration guide in docs/async-migration.md
- [ ] T066 Update README with async usage examples in README.md
- [ ] T067 Update API documentation for all async trait methods in src/
- [ ] T068 Add async examples to doc comments in src/

---

## Phase 8: Polish & Cross-Cutting Concerns

**Purpose**: Final improvements, testing, and validation

- [X] T069 [P] Run cargo fmt on all modified files
- [X] T070 [P] Run cargo clippy and fix all warnings
- [X] T071 [P] Verify all tests pass (unit, integration, contract) with cargo test
- [ ] T072 Performance validation: verify event loop latency ≤16ms in tests/unit/test_performance.rs
- [ ] T073 Performance validation: verify UI responsiveness ≤50ms during async ops in tests/unit/test_performance.rs
- [ ] T074 Performance validation: verify loading indicators appear ≤100ms in tests/unit/test_performance.rs
- [ ] T075 Verify integration with reqwest (test example) in tests/integration/test_library_integration.rs
- [ ] T076 Verify integration with tokio-postgres (test example) in tests/integration/test_library_integration.rs
- [ ] T077 Verify integration with tokio-fs (test example) in tests/integration/test_library_integration.rs
- [ ] T078 Code review: ensure all async patterns follow best practices
- [ ] T079 Update CHANGELOG.md with breaking changes and migration notes
- [ ] T080 Validate quickstart.md examples work correctly

---

## Dependencies & Execution Order

### Phase Dependencies

- **Setup (Phase 1)**: No dependencies - can start immediately
- **Foundational (Phase 2)**: Depends on Setup completion - BLOCKS all user stories
- **User Stories (Phase 3-5)**: All depend on Foundational phase completion
  - User Story 1 (P1): Can start after Foundational - No dependencies on other stories
  - User Story 2 (P1): Can start after Foundational - May use US1 components but independently testable
  - User Story 3 (P2): Can start after Foundational - May use US1/US2 components but independently testable
- **Loading Indicators (Phase 6)**: Depends on Foundational, enhances all user stories
- **Examples & Docs (Phase 7)**: Depends on all user stories being complete
- **Polish (Phase 8)**: Depends on all previous phases

### User Story Dependencies

- **User Story 1 (P1)**: Can start after Foundational (Phase 2) - No dependencies on other stories
- **User Story 2 (P1)**: Can start after Foundational (Phase 2) - Uses cancellation/timeout features, independently testable
- **User Story 3 (P2)**: Can start after Foundational (Phase 2) - Uses background tasks, independently testable

### Within Each User Story

- Tests MUST be written and FAIL before implementation (TDD)
- Trait conversions before implementations
- Core async support before advanced features
- Story complete before moving to next priority

### Parallel Opportunities

- All Setup tasks marked [P] can run in parallel (T002, T003)
- Foundational tasks marked [P] can run in parallel (T006, T007, T009, T010)
- All tests for a user story marked [P] can run in parallel
- Trait conversions marked [P] can run in parallel (T020, T021, T022)
- Different user stories can be worked on in parallel by different team members (after Foundational)
- Example updates can run in parallel (T061, T062, T063)
- Documentation tasks can run in parallel (T065, T066, T067, T068)
- Polish tasks marked [P] can run in parallel (T069, T070, T075, T076, T077)

---

## Parallel Example: User Story 1

```bash
# Launch all contract tests for User Story 1 together:
Task: "Contract test for async DataSource::refresh in tests/contract/test_async_data_source.rs"
Task: "Contract test for async View::handle_event in tests/contract/test_async_view.rs"
Task: "Contract test for async Command execution in tests/contract/test_async_command.rs"

# Launch all trait conversions together:
Task: "Convert DataSource trait to async using async-trait in src/data_source/data_source_trait.rs"
Task: "Convert View trait handle_event to async using async-trait in src/view/view_trait.rs"
Task: "Convert Command execute function to async in src/command/mod.rs"
```

---

## Parallel Example: Foundational Phase

```bash
# Launch foundational tasks in parallel:
Task: "Implement background task spawning in src/app/background_tasks.rs"
Task: "Implement cancellation token support in src/app/background_tasks.rs"
Task: "Implement async terminal event reading in src/app/runtime.rs"
Task: "Add loading indicator tracking to Runtime in src/app/runtime.rs"
```

---

## Implementation Strategy

### MVP First (User Story 1 Only)

1. Complete Phase 1: Setup (T001-T003)
2. Complete Phase 2: Foundational (T004-T013) - **CRITICAL - blocks all stories**
3. Complete Phase 3: User Story 1 (T014-T028)
4. **STOP and VALIDATE**: Test User Story 1 independently
   - Verify async DataSource refresh works
   - Verify async Command execution works
   - Verify UI remains responsive
5. Deploy/demo if ready

### Incremental Delivery

1. Complete Setup + Foundational → Foundation ready
2. Add User Story 1 → Test independently → Validate MVP
3. Add Loading Indicators (Phase 6) → Enhance UX
4. Add User Story 2 → Test independently → Enhanced responsiveness
5. Add User Story 3 → Test independently → Streaming support
6. Update Examples & Docs → Migration ready
7. Polish & Validate → Production ready

### Parallel Team Strategy

With multiple developers:

1. Team completes Setup + Foundational together
2. Once Foundational is done:
   - Developer A: User Story 1 (async service integration)
   - Developer B: User Story 2 (responsive UI) + Loading Indicators
   - Developer C: User Story 3 (streaming) + Examples
3. Stories complete and integrate independently
4. Team collaborates on Documentation and Polish

---

## Notes

- [P] tasks = different files, no dependencies
- [Story] label maps task to specific user story for traceability
- Each user story should be independently completable and testable
- Verify tests fail before implementing (TDD)
- Commit after each task or logical group
- Stop at any checkpoint to validate story independently
- All async operations must be Send + Sync
- Framework manages Tokio runtime internally (no application setup)
- Loading indicators appear automatically (non-negotiable)
- Operations cancelled on view switch (FR-016)
- Timeouts configurable, default 30s (FR-017)

