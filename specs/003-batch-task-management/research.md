# Research: Batch Task Management

**Feature**: Batch Task Management  
**Date**: 2025-01-27  
**Status**: Complete

## Research Questions

### 1. Tokio Concurrency Control Patterns

**Question**: What is the best pattern for implementing concurrency-limited batch task execution in Tokio?

**Research Findings**:
- Tokio provides `tokio::sync::Semaphore` for concurrency control
- `tokio::task::JoinSet` can be used to manage multiple tasks and collect results
- `Semaphore::acquire()` returns a permit that can be held during task execution
- Permits are automatically released when dropped, allowing new tasks to start
- Pattern: Create semaphore with desired limit, acquire permit before spawning each task, hold permit until task completes

**Decision**: Use `tokio::sync::Semaphore` for concurrency control combined with `tokio::task::JoinSet` for task management and result collection.

**Rationale**: 
- Semaphore provides efficient, built-in concurrency limiting
- JoinSet provides clean API for managing multiple tasks and collecting results
- Both are standard Tokio primitives, well-tested and performant
- Permits automatically release on drop, preventing resource leaks

**Alternatives Considered**:
- Manual task tracking with channels: More complex, error-prone
- `futures::stream::Stream` with `buffer_unordered`: Less control over individual task results
- Custom task pool: Unnecessary complexity for this use case

---

### 2. CPU Core Detection in Rust

**Question**: How to detect available CPU cores for default concurrency limit?

**Research Findings**:
- `std::thread::available_parallelism()` provides the number of available CPU cores/threads
- Returns `Result<NonZeroUsize>` - may fail on some platforms
- Standard library function, no external dependencies
- Should handle fallback case (e.g., default to 4 if detection fails)

**Decision**: Use `std::thread::available_parallelism()` with fallback to 4 cores if detection fails. Default concurrency limit = `available_parallelism().unwrap_or(4) * 2`.

**Rationale**:
- Standard library function, no dependencies
- Handles platform differences automatically
- Multiplying by 2 accounts for I/O-bound tasks (common in CLI applications)
- Fallback ensures functionality even if detection fails

**Alternatives Considered**:
- Fixed default (e.g., 10): Less adaptive to system resources
- `num_cpus` crate: External dependency, unnecessary when stdlib provides this
- Single core default: Too conservative for I/O-bound tasks

---

### 3. Error Aggregation and Task Result Collection

**Question**: How to efficiently collect and aggregate results from multiple concurrent tasks?

**Research Findings**:
- `tokio::task::JoinSet` provides `join_next()` for collecting results as they complete
- Results can be collected in completion order (not spawn order) using `join_next()` in a loop
- Each task result should include: identifier, status (success/failure/cancelled), optional value, optional error
- Errors should preserve full context (task identifier, error message, chain)

**Decision**: Use `JoinSet` with custom `TaskResult` enum containing success/failure/cancelled variants. Collect results via `join_next()` loop, maintaining order of completion.

**Rationale**:
- JoinSet handles task lifecycle automatically
- `join_next()` provides completion-order results as required by FR-4
- Custom result type provides type safety and clear status distinction
- Preserves all error information with context

**Alternatives Considered**:
- Channel-based collection: More complex, requires manual coordination
- Collecting in spawn order: Doesn't meet FR-4 requirement for completion order
- Generic result type: Less type-safe, harder to distinguish cancellation from failure

---

### 4. Task Cancellation in Batch Context

**Question**: How to handle cancellation of individual tasks within a batch without affecting others?

**Research Findings**:
- Tokio's `CancellationToken` can be cloned and shared
- Each task in a batch should have its own `CancellationToken`
- Cancellation can be checked via `token.cancelled()` or `token.is_cancelled()`
- Cancelled tasks should return a distinct result variant (not failure)
- `JoinHandle::abort()` can forcefully cancel tasks, but graceful cancellation via token is preferred

**Decision**: Each task in a batch gets its own `CancellationToken`. Tasks check for cancellation and return `TaskResult::Cancelled` status. Batch-level cancellation cancels all tokens, individual task cancellation cancels only that token.

**Rationale**:
- CancellationToken provides clean cancellation API
- Per-task tokens allow independent cancellation
- Graceful cancellation allows tasks to clean up resources
- Distinct cancelled status enables proper aggregation (separate from failures)

**Alternatives Considered**:
- Shared cancellation token: Doesn't allow individual task cancellation
- `JoinHandle::abort()`: Forceful, doesn't allow cleanup
- Channel-based cancellation: More complex, unnecessary overhead

---

### 5. Task Identifier and Error Context

**Question**: How to associate task identifiers with results and errors?

**Research Findings**:
- Task identifiers can be provided as optional `String` or `&str` when creating tasks
- Identifiers should be stored with task metadata and included in result/error structures
- Positional index (usize) can serve as fallback identifier
- Error types should include identifier for context

**Decision**: Accept optional task identifier (String) when creating batch tasks. Store identifier with task metadata. Include identifier in all result types and error messages. Use positional index (0-based) as fallback when identifier not provided.

**Rationale**:
- Optional identifier provides flexibility (applications can use meaningful names)
- Positional index ensures all tasks are identifiable even without explicit identifier
- Including identifier in errors improves debugging experience
- String type is flexible enough for various identifier formats

**Alternatives Considered**:
- Required identifier: Less flexible, adds boilerplate for simple cases
- Generic identifier type: Unnecessary complexity for current use case
- UUID generation: Overkill, applications should provide meaningful identifiers

---

### 6. Default and Maximum Concurrency Limits

**Question**: What are appropriate default and maximum concurrency limit values?

**Research Findings**:
- Default should scale with system resources (CPU cores)
- Maximum should prevent resource exhaustion (fixed value)
- Common patterns: default = CPU cores * 2 (for I/O-bound), max = 100-1000
- CLI applications typically benefit from higher defaults than CPU-bound workloads

**Decision**: Default limit = `available_parallelism().unwrap_or(4) * 2`. Maximum limit = 100. Applications can specify lower limits but cannot exceed maximum.

**Rationale**:
- CPU-based default adapts to system capabilities
- Multiplying by 2 accounts for I/O-bound nature of CLI tasks
- Maximum of 100 prevents resource exhaustion while allowing high concurrency
- Enforcing maximum protects against misconfiguration

**Alternatives Considered**:
- Fixed default (e.g., 10): Less adaptive, may be too low on powerful systems
- Maximum based on system resources: Too complex, may allow excessive concurrency
- No maximum: Risk of resource exhaustion

---

## Summary

All technical decisions have been made based on Tokio best practices and Rust standard library capabilities. The implementation will use:
- `tokio::sync::Semaphore` for concurrency control
- `tokio::task::JoinSet` for task management
- `std::thread::available_parallelism()` for CPU detection
- `CancellationToken` for cancellation support
- Custom result types for type-safe status tracking

No external dependencies beyond existing Tokio ecosystem are required.

