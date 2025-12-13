# Implementation Tasks: CLI Mode Detection

**Feature**: CLI Mode Detection  
**Branch**: `009-cli-mode-detection`  
**Date**: 2025-01-27  
**Spec**: [../009_cli_mode_detection.md](../009_cli_mode_detection.md)  
**Plan**: [plan.md](./plan.md)

## Summary

This document breaks down the CLI mode detection feature into actionable, dependency-ordered tasks organized by user story. Each task is independently implementable and testable.

**Total Tasks**: 48  
**Setup**: 4 tasks  
**Foundational**: 10 tasks  
**User Story 1 (P1)**: 8 tasks  
**User Story 2 (P1)**: 9 tasks  
**User Story 3 (P1)**: 9 tasks  
**Polish**: 8 tasks

## Dependencies

```
Phase 1: Setup
  └─> Phase 2: Foundational (terminal detection, env var support)
      ├─> Phase 3: User Story 1 (color detection)
      ├─> Phase 4: User Story 2 (interactive mode, progress, quiet)
      └─> Phase 5: User Story 3 (output format detection)
          └─> Phase 6: Polish
```

**Story Dependencies**:
- User Story 1 (P1) - Conditional Color Output: Depends on Foundational (needs terminal detection and env var support)
- User Story 2 (P1) - Interactive vs Batch Mode: Depends on Foundational (needs terminal detection and env var support)
- User Story 3 (P1) - Output Format Selection: Depends on Foundational (needs terminal detection and env var support)

**Note**: All three user stories can be implemented in parallel after Foundational phase, as they use different functional requirements.

## Implementation Strategy

**MVP Scope**: Foundational + User Story 1 (Terminal Detection + Color Output) - Provides core detection and color output capabilities.

**Incremental Delivery**:
1. **MVP**: Terminal detection and color output detection (Foundational + User Story 1)
2. **Increment 1**: Interactive mode and progress detection (User Story 2)
3. **Increment 2**: Output format detection (User Story 3)
4. **Final**: Terminal dimensions (optional), integration, documentation

## Phase 1: Setup

**Goal**: Prepare project structure and dependencies for CLI mode detection module.

**Independent Test**: Project compiles and existing tests pass.

### Tasks

- [x] T001 Create cli_mode module directory structure in src/cli_mode/
- [x] T002 Create cli_mode module root file with module declaration in src/cli_mode/mod.rs
- [x] T003 Create test directory structure for cli_mode tests in tests/cli_mode/
- [x] T004 Add cli_mode module re-export to src/lib.rs

## Phase 2: Foundational

**Goal**: Implement core terminal detection and environment variable support needed by all user stories.

**Independent Test**: Terminal detection functions return correct values for stdin/stdout/stderr in TTY and non-TTY environments. Environment variable functions correctly read and parse NO_COLOR, OUTPUT_FORMAT, QUIET, FORCE_COLOR.

**Acceptance Criteria**:
- Applications can check if stdout/stderr/stdin are connected to terminals independently
- Each stream is detected independently; streams may have different TTY states
- Detection works consistently across Windows, Linux, macOS
- When detection fails, utilities return safe defaults (assume non-interactive) without errors
- Environment variables are read and parsed correctly with proper precedence handling

### Tasks

- [x] T005 [P] Implement is_stdout_tty() function using std::io::stdout().is_terminal() in src/cli_mode/mod.rs
- [x] T006 [P] Implement is_stderr_tty() function using std::io::stderr().is_terminal() in src/cli_mode/mod.rs
- [x] T007 [P] Implement is_stdin_tty() function using std::io::stdin().is_terminal() in src/cli_mode/mod.rs
- [x] T008 Implement safe TTY detection wrapper using std::panic::catch_unwind or error handling pattern that catches any panics/errors from TTY detection and returns false (non-interactive default) in src/cli_mode/mod.rs
- [x] T009 [P] Implement read_env_var() helper function for reading environment variables with case-insensitive support in src/cli_mode/mod.rs
- [x] T010 [P] Implement is_no_color_set() function checking NO_COLOR environment variable in src/cli_mode/mod.rs
- [x] T011 [P] Implement is_force_color_set() function checking FORCE_COLOR environment variable in src/cli_mode/mod.rs
- [x] T012 [P] Add unit tests for TTY detection functions (stdin/stdout/stderr) in TTY and non-TTY environments in tests/cli_mode/tty_detection.rs
- [x] T013 [P] Add unit tests for environment variable reading and parsing in tests/cli_mode/environment_variables.rs
- [x] T014 [P] Add unit tests for safe TTY detection wrapper error handling in tests/cli_mode/tty_detection.rs

## Phase 3: User Story 1 - Conditional Color Output (P1)

**Goal**: Enable developers to automatically detect whether output should be colored based on terminal capabilities and environment variables.

**Independent Test**: Given a CLI tool that outputs colored text, a developer can run the tool in an interactive terminal and see colored output, pipe the output to a file and see plain text without color codes, and set environment variables to override color behavior when needed.

**Acceptance Criteria**:
- Applications can check if colors should be enabled for a specific output stream (stdout or stderr)
- Color detection checks the TTY state of the specific stream being used
- NO_COLOR overrides all other settings (including FORCE_COLOR and terminal-based detection)
- When NO_COLOR is not set, FORCE_COLOR can override terminal-based detection
- When neither NO_COLOR nor FORCE_COLOR is set, colors are enabled by default when stream is connected to an interactive terminal
- Colors are disabled when stream is not connected to a terminal and no FORCE_COLOR override is set
- When terminal detection fails, colors are disabled (safe default) without errors

### Tasks

- [x] T015 [US1] Implement should_color_output() function for stdout with NO_COLOR > FORCE_COLOR > TTY precedence in src/cli_mode/mod.rs
- [x] T016 [US1] Implement should_color_stderr() function for stderr with NO_COLOR > FORCE_COLOR > TTY precedence in src/cli_mode/mod.rs
- [x] T017 [US1] Add doc comments with examples for color detection functions in src/cli_mode/mod.rs
- [x] T018 [US1] Add unit tests for color detection with NO_COLOR set (should disable colors) in tests/cli_mode/color_detection.rs
- [x] T019 [US1] Add unit tests for color detection with FORCE_COLOR set and NO_COLOR unset (should enable colors) in tests/cli_mode/color_detection.rs
- [x] T020 [US1] Add unit tests for color detection with neither env var set (TTY-based detection) in tests/cli_mode/color_detection.rs
- [x] T021 [US1] Add unit tests for color detection with detection failure (should return false safely) in tests/cli_mode/color_detection.rs
- [x] T022 [US1] Add integration test for color detection in interactive terminal vs piped output in tests/integration/cli_mode_integration.rs

## Phase 4: User Story 2 - Interactive vs Batch Mode (P1)

**Goal**: Enable developers to detect interactive mode and determine whether to show progress indicators or prompts.

**Independent Test**: Run the tool in an interactive terminal and see prompts and progress indicators. Run the tool with output piped and see JSON output with no prompts. Verify that interactive features are disabled when not in an interactive environment.

**Acceptance Criteria**:
- Applications can check if running in interactive mode (requires both stdin and stdout to be terminals, checked independently)
- Interactive mode detection is used to enable/disable prompts and user input requests
- Non-interactive mode is assumed when either stdin or stdout is not a terminal
- When detection fails for either stream, non-interactive mode is returned as safe default
- Progress indicators are shown by default in interactive terminals
- Progress indicators are suppressed when QUIET environment variable is set
- Progress indicators are suppressed when not running in an interactive terminal
- Quiet mode works independently of terminal type detection

### Tasks

- [x] T023 [US2] Implement is_interactive() function checking both stdin and stdout are TTYs independently in src/cli_mode/mod.rs
- [x] T024 [US2] Implement is_quiet() function checking QUIET environment variable in src/cli_mode/mod.rs
- [x] T025 [US2] Implement should_show_progress() function combining TTY detection and quiet mode check in src/cli_mode/mod.rs
- [x] T026 [US2] Add doc comments with examples for interactive mode and progress detection functions in src/cli_mode/mod.rs
- [x] T027 [US2] Add unit tests for interactive mode detection with both stdin/stdout TTY in tests/cli_mode/interactive_mode.rs
- [x] T028 [US2] Add unit tests for interactive mode detection with one stream non-TTY (should return false) in tests/cli_mode/interactive_mode.rs
- [x] T029 [US2] Add unit tests for quiet mode detection in tests/cli_mode/quiet_mode.rs
- [x] T030 [US2] Add unit tests for progress indicator detection combining TTY and quiet mode in tests/cli_mode/quiet_mode.rs
- [x] T031 [US2] Add integration test for interactive mode detection in different stream combinations in tests/integration/cli_mode_integration.rs

## Phase 5: User Story 3 - Output Format Selection (P1)

**Goal**: Enable developers to automatically select output format (table, JSON, plain) based on execution environment.

**Independent Test**: Run the tool in an interactive terminal and see table-formatted output. Pipe the tool output to another command and see JSON-formatted output. Set an environment variable to override the default format.

**Acceptance Criteria**:
- Applications can determine the preferred output format
- OUTPUT_FORMAT environment variable takes precedence over automatic terminal-based detection
- When OUTPUT_FORMAT is explicitly set, it is used regardless of terminal type
- When OUTPUT_FORMAT is not set, format defaults to table for interactive terminals and JSON for non-interactive environments
- When terminal detection fails or is ambiguous, format defaults to JSON (safe default) without errors
- Invalid format values are handled gracefully (fallback to default based on terminal type)

### Tasks

- [x] T032 [US3] Define OutputFormat enum (Table, Json, Plain) in src/cli_mode/mod.rs
- [x] T033 [US3] Implement OutputFormat::from_env() method parsing OUTPUT_FORMAT environment variable in src/cli_mode/mod.rs
- [x] T034 [US3] Implement OutputFormat::default() method returning Table for TTY, Json for non-TTY in src/cli_mode/mod.rs
- [x] T035 [US3] Implement get_output_format() function with OUTPUT_FORMAT > terminal-based default precedence in src/cli_mode/mod.rs
- [x] T036 [US3] Add doc comments with examples for output format detection functions in src/cli_mode/mod.rs
- [x] T037 [US3] Add unit tests for output format detection with OUTPUT_FORMAT env var set in tests/cli_mode/format_detection.rs
- [x] T038 [US3] Add unit tests for output format detection with OUTPUT_FORMAT unset (TTY-based default) in tests/cli_mode/format_detection.rs
- [x] T039 [US3] Add unit tests for invalid OUTPUT_FORMAT values (should fallback gracefully) in tests/cli_mode/format_detection.rs
- [x] T040 [US3] Add integration test for output format selection in interactive terminal vs piped output in tests/integration/cli_mode_integration.rs

## Phase 6: Polish & Cross-Cutting Concerns

**Goal**: Add optional terminal dimension detection, integrate with existing cli_output module, and complete documentation.

**Independent Test**: Terminal dimension functions return available dimensions when possible, return None gracefully when unavailable. Integration with cli_output module works correctly. All public APIs are documented.

**Acceptance Criteria**:
- Terminal dimension detection is optional and returns None when unavailable
- Partial information is returned when available (width or height independently)
- Integration with existing cli_output module maintains backward compatibility
- All public functions have doc comments with examples
- Cross-platform compatibility verified

### Tasks

- [x] T041 [P] Implement terminal_width() function returning Option<usize> with COLUMNS env var fallback in src/cli_mode/mod.rs
- [x] T042 [P] Implement terminal_height() function returning Option<usize> with ROWS env var fallback in src/cli_mode/mod.rs
- [x] T043 [P] Add unit tests for terminal dimension detection when available, including partial scenarios (only width available with height=None, only height available with width=None) in tests/cli_mode/terminal_dimensions.rs
- [x] T044 [P] Add unit tests for terminal dimension detection when unavailable (should return None) in tests/cli_mode/terminal_dimensions.rs
- [x] T045 Refactor existing cli_output::should_use_color() to use cli_mode::should_color_output() in src/cli_output/mod.rs
- [x] T046 Add module-level documentation with usage examples in src/cli_mode/mod.rs
- [x] T047 Add cross-platform compatibility tests (verify behavior on Windows, Linux, macOS) in tests/cli_mode/tty_detection.rs
- [x] T048 Add integration test for mixed stream states (different TTY states for stdin/stdout/stderr) in tests/integration/cli_mode_integration.rs

