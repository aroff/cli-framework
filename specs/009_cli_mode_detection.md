# Feature Specification: CLI Mode Detection

**Feature Branch**: `009-cli-mode-detection`  
**Created**: 2025-01-27  
**Status**: Draft  
**Input**: Enhancement to provide utilities for CLI vs TUI mode detection

## Purpose

Enable CLI applications built with the framework to automatically detect their execution environment and adapt behavior accordingly. Applications should be able to determine whether they are running in an interactive terminal, whether output should be colored, what output format to use, and whether to show progress indicators or prompts.

This feature addresses the common need for CLI tools to behave differently based on execution context: interactive terminals benefit from colored output, tables, and progress bars, while non-interactive environments (scripts, pipes, CI/CD) require plain text, JSON output, and no interactive prompts.

## Clarifications

### Session 2025-01-27

- Q: When multiple environment variables conflict or when environment variables conflict with automatic detection, what should the precedence order be? → A: Explicit environment variables override automatic detection; NO_COLOR takes precedence over FORCE_COLOR; OUTPUT_FORMAT overrides terminal-based defaults.
- Q: When terminal detection fails or returns ambiguous results (e.g., TTY check throws an exception, terminal info unavailable), what should the framework do? → A: Return safe defaults (non-interactive mode) silently without errors; optional debug logging available for troubleshooting.
- Q: For terminal dimension detection (FR-7, optional), when partial information is available (e.g., width but not height, or vice versa), what should the framework return? → A: Return available dimensions individually; None for unavailable dimensions (e.g., width=Some(80), height=None if only width is available).
- Q: When stdin, stdout, and stderr have different TTY states (e.g., stdout is a TTY but stderr is piped, or stdin is a TTY but stdout is redirected), how should detection utilities behave? → A: Each stream (stdin, stdout, stderr) is detected independently; applications query the specific stream they need for their use case.

## User Scenarios & Testing *(mandatory)*

### User Story 1 - Conditional Color Output (Priority: P1)

A developer building a CLI tool needs to output colored text when running in an interactive terminal, but plain text when output is piped to a file or running in a non-interactive environment. They want the tool to automatically detect the environment and enable or disable colors accordingly.

**Why this priority**: Colored output improves readability in interactive terminals but can cause issues when piped to files or used in scripts. Automatic detection ensures the best experience in both scenarios.

**Current limitation**: The developer must manually check environment variables and terminal capabilities, which is error-prone and inconsistent across different tools.

**Independent Test**:  
Given a CLI tool that outputs colored text, a developer can:
- Run the tool in an interactive terminal and see colored output
- Pipe the output to a file and see plain text without color codes
- Set environment variables to override color behavior when needed

**Acceptance Scenarios**:

1. **Given** a CLI tool running in an interactive terminal, **When** it outputs messages, **Then** colors are enabled by default.
2. **Given** a CLI tool with output piped to a file, **When** it outputs messages, **Then** colors are disabled automatically to avoid color codes in the file.
3. **Given** a user sets the NO_COLOR environment variable, **When** the tool outputs messages, **Then** colors are disabled regardless of terminal type.

---

### User Story 2 - Interactive vs Batch Mode (Priority: P1)

A developer building a CLI tool needs to behave differently in interactive mode (showing prompts, progress bars, confirmations) versus batch mode (JSON output, no prompts, automated responses). They want the tool to detect the mode automatically.

**Why this priority**: Interactive features improve user experience in terminals but break automation and scripting. Automatic detection enables both use cases seamlessly.

**Current limitation**: The developer must manually implement mode detection logic, which is often incomplete or inconsistent.

**Independent Test**:

- Run the tool in an interactive terminal and see prompts and progress indicators
- Run the tool with output piped and see JSON output with no prompts
- Verify that interactive features are disabled when not in an interactive environment

**Acceptance Scenarios**:

1. **Given** a CLI tool running in an interactive terminal, **When** it needs user input, **Then** it shows prompts and waits for responses.
2. **Given** a CLI tool with output piped to another command, **When** it needs to output data, **Then** it uses JSON format suitable for parsing.
3. **Given** a CLI tool running in a non-interactive environment, **When** it would normally show progress, **Then** progress indicators are suppressed.

---

### User Story 3 - Output Format Selection (Priority: P1)

A developer building a CLI tool needs to provide human-readable table output for interactive use and machine-readable JSON output for scripting. They want the tool to automatically select the appropriate format based on the execution environment.

**Why this priority**: Different output formats serve different purposes: tables for human readability, JSON for automation. Automatic selection based on context improves usability.

**Current limitation**: The developer must implement format detection logic or require users to manually specify format, reducing usability.

**Independent Test**:

- Run the tool in an interactive terminal and see table-formatted output
- Pipe the tool output to another command and see JSON-formatted output
- Set an environment variable to override the default format

**Acceptance Scenarios**:

1. **Given** a CLI tool running in an interactive terminal, **When** it outputs data, **Then** it uses table format by default.
2. **Given** a CLI tool with output piped to another command, **When** it outputs data, **Then** it uses JSON format by default for easy parsing.
3. **Given** a user sets the OUTPUT_FORMAT environment variable, **When** the tool outputs data, **Then** it uses the specified format regardless of terminal type.

---

### Edge Cases

1. **Non-TTY Environment**: When running in a non-interactive environment (CI/CD, scripts), the tool defaults to non-interactive behavior (no colors, JSON output, no prompts).
2. **Piped Output**: When output is piped to a file or another command, the tool detects this and disables interactive features.
3. **Environment Variable Override**: Environment variables override automatic detection when set, with explicit precedence: NO_COLOR > FORCE_COLOR for colors; OUTPUT_FORMAT > terminal-based defaults for format.
4. **Missing Terminal Information**: When terminal information cannot be determined or detection fails, the tool assumes non-interactive mode for safety and returns safe defaults (no colors, JSON format, no progress) without throwing errors. Optional debug logging may be available for troubleshooting.
5. **Quiet Mode**: When QUIET environment variable is set, progress indicators and non-essential output are suppressed regardless of terminal type.
6. **Cross-Platform Compatibility**: Detection works consistently across different operating systems (Windows, Linux, macOS).
7. **Mixed Stream States**: When stdin, stdout, and stderr have different TTY states, each stream is detected independently, allowing applications to make stream-specific decisions (e.g., color stdout but not stderr when only stdout is a TTY).

## Functional Requirements

### FR-1: Terminal Type Detection

The framework MUST provide utilities to detect whether the application is running in an interactive terminal.

**Acceptance Criteria**:
- Applications can check if stdout is connected to a terminal independently
- Applications can check if stderr is connected to a terminal independently
- Applications can check if stdin is connected to a terminal independently
- Each stream (stdin, stdout, stderr) is detected independently; streams may have different TTY states
- Applications query the specific stream they need for their use case (e.g., check stdout for output formatting, check stderr for error output formatting)
- Detection works consistently across different operating systems (Windows, Linux, macOS)
- Detection correctly identifies non-interactive environments (pipes, redirects, CI/CD)
- When detection fails or is ambiguous, utilities return safe defaults (assume non-interactive) without throwing errors
- Optional debug logging is available for troubleshooting detection failures

### FR-2: Interactive Mode Detection

The framework MUST provide utilities to determine if the application is running in interactive mode (both input and output are terminals).

**Acceptance Criteria**:
- Applications can check if running in interactive mode (requires both stdin and stdout to be terminals, checked independently)
- Interactive mode detection is used to enable/disable prompts and user input requests
- Non-interactive mode is assumed when either stdin or stdout is not a terminal (each stream checked independently)
- When detection fails or is ambiguous for either stream, non-interactive mode is returned as the safe default without errors

### FR-3: Color Output Detection

The framework MUST provide utilities to determine whether output should be colored based on terminal capabilities and environment variables.

**Acceptance Criteria**:
- Applications can check if colors should be enabled for a specific output stream (stdout or stderr)
- Color detection checks the TTY state of the specific stream being used (stdout for standard output, stderr for error output)
- Environment variables take precedence over automatic detection: NO_COLOR overrides all other settings (including FORCE_COLOR and terminal-based detection)
- When NO_COLOR is not set, FORCE_COLOR can override terminal-based detection to enable colors in non-terminal contexts (if supported)
- When neither NO_COLOR nor FORCE_COLOR is set, colors are enabled by default when the specific output stream is connected to an interactive terminal
- Colors are disabled when the output stream is not connected to a terminal and no FORCE_COLOR override is set
- When terminal detection fails or is ambiguous for the output stream, colors are disabled (safe default) without errors
- Detection respects standard environment variable conventions (NO_COLOR, FORCE_COLOR)

### FR-4: Output Format Detection

The framework MUST provide utilities to determine the preferred output format (table, JSON, plain text) based on environment and user preferences.

**Acceptance Criteria**:
- Applications can determine the preferred output format
- OUTPUT_FORMAT environment variable takes precedence over automatic terminal-based detection
- When OUTPUT_FORMAT is explicitly set, it is used regardless of terminal type
- When OUTPUT_FORMAT is not set, format defaults to table for interactive terminals and JSON for non-interactive environments
- When terminal detection fails or is ambiguous, format defaults to JSON (safe default for non-interactive) without errors
- Invalid format values are handled gracefully (fallback to default based on terminal type)

### FR-5: Progress Indicator Detection

The framework MUST provide utilities to determine whether progress indicators should be shown.

**Acceptance Criteria**:
- Applications can check if progress indicators should be displayed
- Progress indicators are shown by default in interactive terminals
- Progress indicators are suppressed when QUIET environment variable is set
- Progress indicators are suppressed when not running in an interactive terminal
- When terminal detection fails or is ambiguous, progress indicators are suppressed (safe default) without errors
- Detection considers both terminal type and quiet mode preference

### FR-6: Quiet Mode Detection

The framework MUST provide utilities to detect if the application is running in quiet mode.

**Acceptance Criteria**:
- Applications can check if quiet mode is enabled
- Quiet mode is determined by QUIET environment variable
- When quiet mode is enabled, non-essential output and progress indicators are suppressed
- Quiet mode works independently of terminal type detection

### FR-7: Terminal Dimensions (Optional)

The framework MAY provide utilities to determine terminal dimensions (width, height) when available.

**Acceptance Criteria**:
- Applications can attempt to get terminal width and height independently
- Dimensions are only available when running in an interactive terminal
- When a dimension cannot be determined, that specific dimension returns no value (not an error), while the other dimension may still be available
- Partial information is returned when available (e.g., if only width is available, width returns Some(value) and height returns None)
- Dimensions can be used to optimize output formatting (table width, pagination)
- Fallback behavior is graceful when dimensions are unavailable (applications handle None values appropriately)

### FR-8: Environment Variable Support

The framework MUST support standard environment variables for controlling behavior.

**Acceptance Criteria**:
- Environment variables take precedence over automatic detection in the following order: NO_COLOR (highest priority for colors), OUTPUT_FORMAT (overrides terminal-based format detection), QUIET, FORCE_COLOR (lowest priority for colors, only applies when NO_COLOR is not set)
- NO_COLOR environment variable disables color output when set, overriding all other color settings
- OUTPUT_FORMAT environment variable specifies output format preference (table, json, plain) and overrides terminal-based defaults
- QUIET environment variable enables quiet mode when set
- FORCE_COLOR environment variable can enable colors in non-terminal contexts (if supported), but only when NO_COLOR is not set
- Environment variable values are case-insensitive where appropriate
- Missing or invalid environment variable values are handled gracefully

## Success Criteria

1. **Automatic Adaptation**: CLI applications automatically adapt their behavior (colors, format, interactivity) based on execution environment in 100% of common scenarios (interactive terminal, piped output, CI/CD environments).

2. **Developer Productivity**: Developers can implement environment-aware behavior with 80% less code compared to manual detection logic (measured by lines of code reduction in example implementations).

3. **User Experience**: Users see appropriate output format and behavior without manual configuration in 95% of use cases (interactive terminals show tables and colors, scripts receive JSON output).

4. **Script Compatibility**: CLI tools work correctly in automation scenarios (pipes, redirects, CI/CD) without requiring special flags or configuration, enabling seamless integration into scripts and pipelines.

5. **Cross-Platform Consistency**: Detection utilities work consistently across Windows, Linux, and macOS, with behavior differences only where platform capabilities differ.

6. **Environment Variable Compliance**: Framework respects standard environment variables (NO_COLOR, OUTPUT_FORMAT, QUIET) following industry conventions, ensuring compatibility with user expectations and other tools.

## Key Entities

### Execution Context
The environment in which a CLI application is running, including terminal capabilities, input/output connections, and user preferences. This context determines how the application should behave (interactive vs non-interactive, colored vs plain, table vs JSON).

### Terminal Type
Classification of the input/output connections: interactive terminal (TTY), piped output, redirected output, or non-interactive environment. This classification drives automatic behavior selection.

### Output Format Preference
User or environment-specified preference for output format: table (human-readable), JSON (machine-readable), or plain text. This preference can be explicitly set via environment variable or automatically determined based on terminal type.

### Interactive Mode
State indicating whether the application can interact with the user through prompts and input requests. Interactive mode requires both stdin and stdout to be connected to terminals.

## Assumptions

1. **Standard Environment Variables**: Users and automation tools follow standard conventions for environment variables (NO_COLOR, OUTPUT_FORMAT, QUIET), which are widely adopted in the CLI tool ecosystem.

2. **Terminal Capabilities**: When running in a terminal, the terminal supports basic capabilities (ANSI colors, standard input/output). The framework handles cases where capabilities are limited gracefully.

3. **Non-Interactive Default**: When terminal information cannot be determined or is ambiguous, the framework assumes non-interactive mode for safety and compatibility with automation.

4. **User Preferences**: Users may override automatic detection via environment variables when needed for specific use cases or preferences.

5. **Cross-Platform Differences**: Some platform-specific differences in terminal detection are expected and handled appropriately (Windows vs Unix-like systems).

## Dependencies

- Standard library capabilities for terminal detection (TTY checks)
- Environment variable access
- Existing output formatting utilities (for format-aware output)

## Out of Scope

- Terminal emulation or terminal feature detection beyond basic TTY checks
- Advanced terminal capabilities (cursor positioning, advanced formatting beyond colors)
- Terminal size detection beyond basic width/height (this is optional and may return no value)
- Custom terminal protocols or non-standard terminal types
- Terminal theme detection or color scheme management
- Interactive terminal UI features (covered by TUI framework features)

## Testing Requirements

### Test Scenarios

1. **TTY Detection**: Verify terminal type detection works correctly in TTY and non-TTY environments.

2. **Color Detection**: Test color output detection with various environment variable combinations (NO_COLOR set/unset, FORCE_COLOR set/unset, TTY/non-TTY).

3. **Output Format Detection**: Test format selection with OUTPUT_FORMAT environment variable and automatic defaults based on terminal type.

4. **Interactive Mode Detection**: Test interactive mode detection with different stdin/stdout combinations (both TTY, one TTY, neither TTY).

5. **Quiet Mode Detection**: Test quiet mode detection and its interaction with progress indicator display.

6. **Environment Variable Handling**: Test all supported environment variables with valid and invalid values, ensuring graceful handling.

7. **Cross-Platform Compatibility**: Verify behavior consistency across Windows, Linux, and macOS platforms.

8. **Piped Output Detection**: Verify that piped output is correctly detected and triggers appropriate behavior (no colors, JSON format).

9. **Terminal Dimensions**: Test terminal dimension detection when available and graceful handling when unavailable.

## Backward Compatibility

New functionality is additive. Existing applications are not affected unless they opt-in to use the new detection utilities. Applications can adopt environment-aware behavior incrementally without modifying existing code.

## Related Components

- CLI output utilities (006_cli_output_utilities.md) for format-aware output formatting
- Progress reporting (004_progress_reporting.md) for conditional progress display based on terminal type
- Message formatting that respects color preferences
