# API Contracts: Task Result Aggregation

**Feature**: Task Result Aggregation  
**Date**: 2025-01-27  
**Phase**: 1 - Design & Contracts

## Overview

This document defines the API contracts for task result aggregation utilities. These utilities extend the existing `BatchResult` type from batch task management with convenience methods and provide standalone aggregation functions.

## Type Contracts

### BatchResult Extensions

The existing `BatchResult` type is extended with new methods for aggregation utilities.

```rust
impl BatchResult {
    /// Generate a formatted summary message
    ///
    /// Creates a human-readable summary of the batch operation results,
    /// including total count, success/failure/cancelled counts, and success rate.
    ///
    /// # Returns
    /// Formatted string summary. Examples:
    /// - "No tasks executed" (when total == 0)
    /// - "All 45 tasks completed successfully" (when all succeeded)
    /// - "Completed 45 tasks: 42 successful, 3 failed (93.3% success rate)"
    ///
    /// # Behavior
    /// - Uses custom_summary if set, otherwise generates auto-summary
    /// - Handles all edge cases (empty, all success, all failure, all cancelled)
    /// - Includes cancelled count in summary when > 0
    pub fn generate_summary(&self) -> String;

    /// Format errors for display with task identification
    ///
    /// Creates a formatted string listing all errors with task identification
    /// and clear numbering for user-friendly display.
    ///
    /// # Returns
    /// Formatted string with numbered error list. Format:
    /// ```
    /// Errors (3):
    ///   1. [file: image.jpg] Failed to process: permission denied
    ///   2. [Task 5] Network error: connection timeout
    ///   3. [file: data.json] Parse error: invalid JSON
    /// ```
    ///
    /// # Behavior
    /// - Returns empty string if no errors
    /// - Includes task identifier in each error line
    /// - Uses provided identifier if available, otherwise positional index
    /// - Formats TaskIdentifier::Provided as "[identifier]"
    /// - Formats TaskIdentifier::Index as "[Task N]"
    pub fn format_errors(&self) -> String;

    /// Set a custom summary message
    ///
    /// Builder method to override the auto-generated summary message.
    ///
    /// # Arguments
    /// * `summary` - Custom summary message
    ///
    /// # Returns
    /// New `BatchResult` instance with custom summary set
    ///
    /// # Behavior
    /// - Creates a new instance with custom_summary set
    /// - Custom summary takes precedence over auto-generated summary
    pub fn with_summary(self, summary: impl Into<String>) -> Self;
}
```

**Contract Details**:
- All methods are non-mutating (except `with_summary` which returns new instance)
- `generate_summary()` respects custom_summary if set
- `format_errors()` always includes task identification
- Error formatting handles both `TaskIdentifier` variants appropriately

---

## Aggregation Utility Functions

### Basic Aggregation

```rust
/// Aggregate task results into summary statistics
///
/// Takes a collection of task results and creates an aggregated `BatchResult`
/// with statistics and error collection.
///
/// # Arguments
/// * `results` - Collection of task results from batch operations
///
/// # Returns
/// `BatchResult` with aggregated statistics
///
/// # Behavior
/// - Counts successful, failed, and cancelled tasks
/// - Collects all errors from failed tasks
/// - Calculates success rate (excluding cancelled tasks)
/// - Preserves task identification in error collection
pub fn aggregate_results(results: Vec<BatchTaskResult>) -> BatchResult;
```

**Contract Details**:
- Handles empty result collections gracefully
- Preserves all errors with task identification
- Calculates success rate correctly (successful / (successful + failed))
- Excludes cancelled tasks from success rate calculation

---

### Aggregation with Error Filtering

```rust
/// Aggregate results with custom error filtering
///
/// Filters errors based on a predicate before including in aggregation.
/// Filtered errors are excluded from failure counts but preserved separately
/// for auditability.
///
/// # Arguments
/// * `results` - Collection of task results from batch operations
/// * `error_filter` - Predicate function that returns true for errors to include in failure count
///
/// # Returns
/// `BatchResult` with:
/// - `failed` count includes only non-filtered errors
/// - `errors` collection contains only non-filtered errors
/// - `filtered_errors` collection contains filtered errors
/// - Success rate calculated based on non-filtered failures
///
/// # Behavior
/// - Errors where filter returns `true` are counted as failures
/// - Errors where filter returns `false` are excluded from failure count but collected in `filtered_errors`
/// - Filtered errors don't affect success rate calculation
/// - All errors (filtered and non-filtered) are preserved for auditability
pub fn aggregate_with_filter<F>(
    results: Vec<BatchTaskResult>,
    error_filter: F,
) -> BatchResult
where
    F: Fn(&anyhow::Error) -> bool;
```

**Contract Details**:
- Filter predicate receives `&anyhow::Error` for each failed task
- Filtered errors are preserved in separate collection
- Success rate excludes filtered errors from calculation
- Both error collections are accessible for reporting

---

### Aggregation with Error Collection Limits

```rust
/// Aggregate results with optional error collection limit
///
/// Collects errors up to a specified limit for memory efficiency with
/// very large batches. Indicates if truncation occurred.
///
/// # Arguments
/// * `results` - Collection of task results from batch operations
/// * `max_errors` - Optional limit on error collection (None = unlimited)
///
/// # Returns
/// `BatchResult` with:
/// - `errors` collection limited to `max_errors` if specified
/// - `truncated` flag set to `true` if errors were truncated
///
/// # Behavior
/// - If `max_errors` is `None`, collects all errors (default behavior)
/// - If `max_errors` is `Some(limit)`, collects first N errors
/// - Sets `truncated = true` if more errors exist than limit
/// - Statistics (counts, success rate) are always accurate regardless of truncation
pub fn aggregate_with_limit(
    results: Vec<BatchTaskResult>,
    max_errors: Option<usize>,
) -> BatchResult;
```

**Contract Details**:
- Error limit applies only to error collection, not to statistics
- Truncation indicator allows applications to know if complete error info is available
- Statistics remain accurate even when errors are truncated
- Default behavior (None) collects all errors

---

### Result Merging

```rust
/// Merge multiple batch results into a single aggregated result
///
/// Combines results from multiple batch operations into a single
/// aggregated result with combined statistics and error collections.
///
/// # Arguments
/// * `results` - Slice of `BatchResult` to merge
///
/// # Returns
/// New `BatchResult` with:
/// - Combined counts (total, successful, failed, cancelled)
/// - Merged error collections from all batches
/// - Recalculated success rate from combined statistics
/// - All errors preserved with task identification
///
/// # Behavior
/// - Combines all counts by summing
/// - Merges all error collections (preserves all errors)
/// - Recalculates success rate from combined counts
/// - Handles empty slice gracefully (returns empty result)
pub fn merge_results(results: &[BatchResult]) -> BatchResult;
```

**Contract Details**:
- Merges up to 100 batch results (per SC-007)
- Preserves all errors from all batches
- Recalculates success rate from combined statistics
- No duplicate detection needed (errors are from different batches)

---

## Error Handling

All aggregation functions use standard Rust error handling:

- **Input Validation**: Functions validate input and handle edge cases gracefully
- **Error Types**: Uses `anyhow::Error` for error information (already in use)
- **Panic Behavior**: Functions should not panic on valid input; edge cases return appropriate results

## Performance Contracts

- **Summary Generation**: Completes in under 10ms for batches up to 10,000 tasks (SC-005)
- **Error Formatting**: Linear time complexity O(n) where n is number of errors
- **Aggregation**: Linear time complexity O(n) where n is number of results
- **Merging**: Linear time complexity O(n) where n is total number of errors across all batches

## Backward Compatibility

All new functionality is additive:
- Existing `BatchResult` fields and methods remain unchanged
- New methods are added via `impl` blocks
- Standalone functions are in the same module namespace
- No breaking changes to existing API

## Usage Examples

See [quickstart.md](../quickstart.md) for detailed usage examples and patterns.

