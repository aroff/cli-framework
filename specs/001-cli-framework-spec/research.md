# Research: CLI Framework – Opinionated TUI Library

**Date**: 2025-11-17  
**Feature**: 001-cli-framework-spec

## Technology Choices

### Decision: Rust with ratatui and crossterm

**Rationale**: 
- Rust provides memory safety, performance, and excellent ecosystem for systems programming
- `ratatui` (formerly `tui-rs`) is the de facto standard for TUI rendering in Rust, actively maintained with good documentation
- `crossterm` provides cross-platform terminal I/O and event handling (Linux, macOS, Windows)
- Both libraries are mature, well-tested, and have active communities

**Alternatives considered**:
- **C/C++ with ncurses**: Lower-level, more complex, less safe
- **Go with tview**: Go's runtime overhead and GC pauses not ideal for responsive TUI
- **Python with rich/textual**: Performance concerns for real-time terminal updates, dependency management complexity
- **Other Rust TUI libraries**: `ratatui` has the best balance of features, performance, and maintainability

### Decision: Single-threaded synchronous event loop for v1

**Rationale**:
- Simpler mental model and implementation
- Sufficient for v1 requirements (monitoring and operations console)
- Easy to embed alongside REST/gRPC servers (applications can use separate threads/runtimes)
- Can evolve to async/background jobs in v2+ without breaking API

**Alternatives considered**:
- **Tokio async runtime**: Adds complexity, not needed for v1's blocking I/O model
- **Multi-threaded event loop**: Unnecessary complexity for single-service CLI use case

### Decision: OpenTelemetry for observability

**Rationale**:
- Industry standard for observability (logs, metrics, traces)
- Framework-agnostic, applications can use any OpenTelemetry-compatible backend
- Optional integration keeps framework lightweight for applications that don't need it
- Aligns with modern observability practices

**Alternatives considered**:
- **Custom logging/metrics API**: Would require framework to maintain its own observability stack
- **Tracing crate only**: Less comprehensive than OpenTelemetry (no metrics, limited backend options)

## Architecture Patterns

### Decision: Builder pattern for AppBuilder

**Rationale**:
- Fluent API makes configuration readable and discoverable
- Type-safe configuration (compiler catches errors)
- Common Rust pattern, familiar to Rust developers
- Allows incremental configuration building

**Alternatives considered**:
- **Struct with public fields**: Less ergonomic, no validation at construction time
- **Configuration file**: Adds parsing complexity, less type-safe

### Decision: Trait-based abstractions (View, DataSource)

**Rationale**:
- Rust's trait system provides zero-cost abstractions
- Applications implement traits without framework knowing concrete types
- Enables testing with mock implementations
- Clear contract between framework and applications

**Alternatives considered**:
- **Enum-based views**: Less flexible, harder to extend
- **Callback functions**: Less type-safe, harder to test

### Decision: View state persistence (created once, persist across switches)

**Rationale**:
- Better UX: users maintain scroll position, selection, filters when switching views
- Matches user expectations from modern TUI applications
- Efficient: no recreation overhead
- Applications can implement refresh logic without losing state

**Alternatives considered**:
- **Recreate views on switch**: Poor UX, loses user context
- **Lazy creation**: Adds complexity, doesn't solve state persistence issue

## Keybinding and Command Patterns

### Decision: View-specific bindings override global; modals override everything

**Rationale**:
- Context-aware behavior: modals need to capture all input
- View-specific actions take precedence in their context
- Predictable priority order reduces confusion
- Common pattern in TUI applications (vim, tmux, etc.)

**Alternatives considered**:
- **Global always wins**: Breaks context-aware interactions
- **Last-registered wins**: Non-deterministic, hard to debug

### Decision: Standard command syntax (`:command arg=value positional`)

**Rationale**:
- Familiar to users of vim, tmux, and other TUI tools
- Simple to parse (no quoting needed in v1)
- Flexible: supports both named and positional arguments
- Applications can validate and provide helpful error messages

**Alternatives considered**:
- **JSON syntax**: Too verbose for command palette
- **Shell-like syntax**: Requires complex parsing, quoting issues

## Error Handling and Resilience

### Decision: Configurable retry policies and timeout handling in framework

**Rationale**:
- Network operations are common failure point
- Framework can provide sensible defaults while allowing customization
- Consistent error handling across all network operations
- Applications don't need to implement retry logic themselves

**Alternatives considered**:
- **Application responsibility**: Duplicates retry logic across applications
- **Fixed retry policy**: Too rigid, doesn't fit all use cases

### Decision: AppMessage model (short for status bar, details for modal)

**Rationale**:
- Two-tier messaging matches TUI constraints (limited status bar space)
- Users can get quick feedback, then drill into details if needed
- Consistent UX across all framework operations
- Applications can use same model for their own messages

**Alternatives considered**:
- **Single message format**: Doesn't account for space constraints
- **Application-defined messages**: Inconsistent UX across applications

## UI/UX Patterns

### Decision: Mandatory standard empty states and loading indicators

**Rationale**:
- Consistent UX across all applications using the framework
- Reduces application developer burden (don't need to implement these)
- Professional appearance out of the box
- Applications can still customize if needed (via View trait)

**Alternatives considered**:
- **Optional empty/loading UI**: Inconsistent UX, applications might forget to implement
- **Application responsibility**: Duplicates work, inconsistent results

### Decision: Minimum terminal size 80x24 with graceful degradation

**Rationale**:
- 80x24 is standard terminal size (VT100 legacy)
- Graceful degradation ensures functionality even on constrained terminals
- Framework handles layout adjustments automatically
- Applications don't need terminal size detection logic

**Alternatives considered**:
- **Fixed minimum with error**: Too rigid, breaks on smaller terminals
- **No minimum**: Complex layout logic, poor UX on very small terminals

## Data Handling

### Decision: DataSource trait with len(), get(), refresh() API

**Rationale**:
- Supports both in-memory and paginated backends uniformly
- Simple interface, easy to implement
- Framework handles navigation, applications handle data fetching
- Clear separation of concerns

**Alternatives considered**:
- **Iterator-based**: Less flexible for pagination, harder to implement
- **Callback-based**: More complex, harder to test

### Decision: No hard limits on data volume; optimize for typical use cases

**Rationale**:
- Framework remains flexible for various application needs
- Applications with larger datasets can implement server-side pagination
- Performance targets (thousands of rows, tens of thousands of log lines) are reasonable defaults
- Avoids premature optimization

**Alternatives considered**:
- **Hard limits**: Too restrictive, breaks legitimate use cases
- **Unbounded optimization**: Unrealistic, can't optimize for everything

## Security

### Decision: Optional built-in authentication (opt-in)

**Rationale**:
- Some applications need authentication, others don't
- Framework provides common patterns (login, token management, RBAC) without forcing them
- Applications can still use AppContext for custom auth if needed
- Reduces application developer burden for common auth scenarios

**Alternatives considered**:
- **Mandatory authentication**: Too restrictive, breaks simple use cases
- **No authentication support**: Applications must implement from scratch

## Summary

All technology choices and architectural patterns are based on:
1. **Rust ecosystem best practices**: Using mature, well-maintained crates
2. **TUI application patterns**: Following conventions from successful TUI tools
3. **Developer experience**: Minimizing application developer burden while maintaining flexibility
4. **Performance**: Meeting responsiveness targets without over-engineering
5. **Extensibility**: Allowing evolution to v2+ features (async, advanced features) without breaking changes

