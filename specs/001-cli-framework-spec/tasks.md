# Task Checklist: CLI Framework – Opinionated TUI Library

**Feature**: `001-cli-framework-spec`
**Status**: Pending

## Phase 1: Setup

- [X] T001 Initialize project structure (src/app, src/view, src/widget, etc.) in `src/`
- [X] T002 Configure Cargo.toml with dependencies (ratatui, crossterm, anyhow, serde, opentelemetry) in `Cargo.toml`
- [X] T003 Create lib.rs with module definitions and public re-exports in `src/lib.rs`
- [X] T004 Set up test infrastructure (unit, integration, contract test directories) in `tests/`
- [X] T004a [P] Create TestBackend helper utilities for headless TUI testing in `tests/helpers/test_backend.rs`
- [X] T005 Create examples directory structure (simple, multi_view, kitchen_sink) in `examples/`

## Phase 2: Foundational

- [X] T006 [P] Define `AppBuilder` struct and basic methods (new, build) in `src/app/builder.rs`
- [X] T007 [P] Define `AppContext` trait and helpers in `src/app/context.rs`
- [X] T008 [P] Implement basic event loop and runtime in `src/app/runtime.rs`
- [X] T009 [P] Define `AppMessage` and `AppMessageKind` models in `src/message/model.rs`
- [X] T010 [P] Define `Theme` struct with default ANSI colors in `src/view/theme.rs`
- [X] T010a [P] Create unit tests verifying Theme uses standard ANSI 16-color palette and fallback behavior in `tests/unit/view/theme.rs`
- [X] T011 [P] Define `KeyBinding`, `KeymapConfig`, and basic registry in `src/keymap/mod.rs`
- [X] T012a [P] Create contract test structure for View trait in `tests/contract/view_trait.rs` (Test-First: write tests before T012)
- [X] T012 [P] Define `View` trait in `src/view/trait.rs`
- [X] T013a [P] Create contract test structure for DataSource trait in `tests/contract/data_source_trait.rs` (Test-First: write tests before T013)
- [X] T013 [P] Define `DataSource` trait in `src/data_source/trait.rs`
- [X] T014 [P] Define `Command` struct and `CommandArgs` in `src/command/mod.rs`
- [X] T014a [P] Set up contract test infrastructure (test runner, trait contract validators) in `tests/contract/mod.rs` for View, DataSource, Module, and AppBuilder traits (as per contracts/ directory)

## Phase 3: User Story 1 - Build a TUI console for my service quickly (P1)

**Goal**: Enable developers to wire views and datasources to create a functional TUI.

**Independent Test**: `examples/simple` runs and displays a grid view with data.

- [X] T015 [P] [US1] Implement `ViewRegistry` to manage registered views in `src/view/registry.rs`
- [X] T016 [P] [US1] Implement `AppBuilder::register_view` and `map_view_slot` in `src/app/builder.rs`
- [X] T017 [US1] Implement `GridView` widget using `DataSource` trait in `src/widget/grid.rs`
- [X] T018 [US1] Implement `StatusBar` widget in `src/widget/status_bar.rs`
- [X] T019 [US1] Implement `HelpOverlay` widget in `src/widget/help.rs`
- [X] T020 [US1] Wire up `render` loop to draw active view, status bar, and overlays in `src/app/runtime.rs`
- [X] T021 [US1] Implement key handling for F-keys (view switching) and `?` (help) in `src/app/runtime.rs`
- [X] T022 [US1] Create `examples/simple/main.rs` to verify basic TUI loop and GridView

## Phase 4: User Story 2 - Operate a service via commands and keybindings (P1)

**Goal**: Enable command execution via palette and keybindings.

**Independent Test**: `examples/with_commands` allows executing commands via `:` and keys.

- [X] T023 [P] [US2] Implement `CommandRegistry` in `src/command/registry.rs`
- [X] T024 [P] [US2] Implement `AppBuilder::register_command` in `src/app/builder.rs`
- [X] T025 [US2] Implement command parser (`:command arg=value`) in `src/command/parser.rs`
- [X] T026 [US2] Implement `CommandPalette` widget in `src/command/palette.rs`
- [X] T027 [US2] Integrate command palette into runtime (toggle with `:`) in `src/app/runtime.rs`
- [X] T028 [US2] Implement `KeymapResolver` (view > global, modal > all) in `src/keymap/resolver.rs`
- [X] T029 [US2] Connect key events to `Command::execute` via `AppContext` in `src/app/runtime.rs`
- [X] T030 [US2] Implement `ModalView` for detailed feedback/errors in `src/widget/modal.rs`
- [X] T031 [US2] Create `examples/with_commands/main.rs` to verify command execution and palette

## Phase 5: User Story 3 - Inspect live logs and filter issues (P2)

**Goal**: Enable log streaming and filtering.

**Independent Test**: `LogView` correctly displays streaming lines and filters them.

- [X] T032 [P] [US3] Implement `LogView` widget with internal buffer in `src/widget/log.rs`
- [X] T033 [US3] Implement scrolling and "follow mode" logic in `src/widget/log.rs`
- [X] T034 [US3] Implement simple substring filtering in `LogView` in `src/widget/log.rs`
- [X] T035 [US3] Add `LogDataSource` trait or pattern for pushing logs in `src/data_source/log.rs`
- [X] T036 [US3] Integrate `LogView` into `examples/kitchen_sink/main.rs` (reference app)

## Phase 6: User Story 4 - Customize keybindings and UI features (P3)

**Goal**: Allow configuration of UI elements and keybindings.

**Independent Test**: Configuration changes (e.g., hiding status bar) are reflected in the app.

- [X] T037 [P] [US4] Implement `AppBuilder` methods for UI toggles (`with_status_bar`, etc.) in `src/app/builder.rs`
- [X] T038 [US4] Respect UI toggles in `runtime.rs` render loop in `src/app/runtime.rs`
- [X] T039 [P] [US4] Implement `AppBuilder::configure_keymap` in `src/app/builder.rs`
- [X] T040 [US4] Verify custom keybindings override defaults in `tests/keymap.rs`

## Phase 7: Cross-Cutting Concerns & Polish

- [X] T041 [P] Implement retry policies and timeout handling in `src/retry/mod.rs`
- [X] T042 [P] Implement optional authentication hooks in `src/auth/mod.rs`
- [X] T043 [P] Implement OpenTelemetry integration in `src/observability/mod.rs`
- [X] T044 [P] Implement standard empty/loading states in `src/widget/empty_state.rs`
- [X] T045a [P] Create contract test structure for Module trait in `tests/contract/module_trait.rs` (Test-First: write tests before T045)
- [X] T045 [P] Define `Module` trait for internal modularization in `src/app/module.rs`
- [X] T046 Ensure terminal size limits (80x24) are handled gracefully (scrollable content, truncated labels, minimum functional area preserved) in `src/app/runtime.rs`
- [X] T047 Finalize `examples/kitchen_sink` as the comprehensive reference application in `examples/kitchen_sink/main.rs`
- [X] T048 Run all contract tests (view_trait.rs, data_source_trait.rs, module_trait.rs, app_builder_api.rs) and ensure compliance in `tests/contract/`
- [ ] T049 [Optional] Create usability testing guidance document (methodology, sample questions, success criteria for SC-003) in `docs/usability-testing.md`

## Implementation Strategy

- **MVP**: Phases 1-3 provide a working read-only TUI (View, Grid, Navigation).
- **Interactive**: Phase 4 adds interactivity (Commands, Keybindings).
- **Observability**: Phase 5 adds logging capabilities.
- **Production-Ready**: Phases 6-7 add customization, resilience, and polish.

## Dependencies

- **US1** depends on Setup and Foundational tasks.
- **US2** depends on US1 (needs runtime/views to trigger commands).
- **US3** depends on US1 (LogView is a View).
- **US4** depends on US1/US2 (configuring existing features).

