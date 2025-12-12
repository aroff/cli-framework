# Research: Task Result Aggregation

**Feature**: Task Result Aggregation  
**Date**: 2025-01-27  
**Status**: Complete

## Research Questions

### 1. Integration with Existing BatchResult Type

**Question**: How should aggregation utilities integrate with the existing `BatchResult` type from batch task management?

**Research Findings**:
- `BatchResult` already exists in `src/app/background_tasks.rs` with all required fields:
  - `total`, `successful`, `failed`, `cancelled` counts
  - `success_rate` calculation
  - `errors: Vec<(TaskIdentifier, anyhow::Error)>`
  - `results: Vec<BatchTaskResult>`
- The existing type already provides basic aggregation from batch operations
- Need to add convenience methods for formatting and additional aggregation utilities

**Decision**: Extend `BatchResult` with convenience methods (impl blocks) and create standalone aggregation utility functions in the same module. This maintains consistency with existing code while providing flexible usage patterns.

**Rationale**:
- No need to create a new type - `BatchResult` already has all required data
- Extending existing type maintains API consistency
- Standalone functions provide flexibility for custom aggregation scenarios
- Follows Rust best practices: extend types with impl blocks, provide utility functions

**Alternatives Considered**:
- Create new `TaskBatchResult` type: Unnecessary duplication, breaks consistency
- Separate aggregation module: Adds complexity, existing type already suitable
- Wrapper type around `BatchResult`: Unnecessary indirection

---

### 2. Error Filtering Implementation Pattern

**Question**: How to implement error filtering that excludes errors from failure counts but preserves them for auditability?

**Research Findings**:
- Filtered errors need separate collection to distinguish from "real" failures
- Filter predicate should be a closure `Fn(&anyhow::Error) -> bool`
- Filtered errors should not affect success rate calculation
- Both collections (errors and filtered_errors) should be accessible

**Decision**: Add `filtered_errors: Vec<(TaskIdentifier, anyhow::Error)>` field to aggregation result. Provide `aggregate_with_filter()` function that takes a filter predicate. Filtered errors are excluded from `failed` count but collected separately.

**Rationale**:
- Separate collection maintains clear distinction between real failures and filtered errors
- Preserves complete auditability as required by clarifications
- Filter predicate closure provides flexibility for various filtering criteria
- Matches Rust patterns: functional filtering with closures

**Alternatives Considered**:
- Single collection with flags: Less clear, harder to query
- Completely discard filtered errors: Loses auditability requirement
- Count filtered as successful: Misleading statistics

---

### 3. Error Collection Limits for Large Batches

**Question**: How to implement opt-in error collection limits for memory efficiency with very large batches?

**Research Findings**:
- Default behavior: collect all errors (no limits)
- For very large batches (100,000+ tasks), memory may be a concern
- Applications should opt-in to limits when needed
- When limit is applied, should indicate if truncation occurred

**Decision**: Add optional `max_errors: Option<usize>` parameter to aggregation functions. When `Some(limit)`, collect only first N errors and track if truncation occurred. Add `truncated: bool` field to result to indicate if errors were truncated.

**Rationale**:
- Opt-in design provides flexibility: small batches get complete info, large batches can limit
- Optional parameter with default `None` maintains backward compatibility
- Truncation indicator allows applications to know if complete error info is available
- Follows Rust patterns: `Option<T>` for optional configuration

**Alternatives Considered**:
- Automatic limits based on batch size: Less flexible, may truncate when not needed
- Always limit: Breaks requirement for complete error collection by default
- Separate limit function: More complex API, opt-in parameter is cleaner

---

### 4. Error Formatting with Task Identification

**Question**: How to format errors with task identification for user-friendly display?

**Research Findings**:
- `TaskIdentifier` is already part of error collection: `Vec<(TaskIdentifier, anyhow::Error)>`
- `TaskIdentifier` enum has variants: `Provided(String)` or `Index(usize)`
- Error formatting should include identifier in each error line
- Format should be readable: "1. [file: image.jpg] Error message" or "1. [Task 0] Error message"

**Decision**: Create `format_errors()` method on `BatchResult` that iterates over `errors` collection, formats each with its `TaskIdentifier`, and produces numbered list. Use pattern matching on `TaskIdentifier` to format appropriately.

**Rationale**:
- Task identification already available in error collection
- Numbered list provides clear structure for multiple errors
- Pattern matching on enum provides type-safe formatting
- Consistent with existing error collection structure

**Alternatives Considered**:
- Separate formatting function: Less discoverable, method on type is more idiomatic
- Omit task identification: Breaks requirement for clear context
- Different format: Current format is clear and readable

---

### 5. Summary Message Generation

**Question**: How to generate user-friendly summary messages from aggregated results?

**Research Findings**:
- Summary should include: total, successful, failed, cancelled counts, success rate
- Should handle edge cases: empty results, all success, all failure, all cancelled
- Should support custom message override
- Format should be clear and readable

**Decision**: Create `generate_summary()` method on `BatchResult` that generates formatted string based on result state. Support custom summary via `with_summary()` builder method. Handle all edge cases with appropriate messages.

**Rationale**:
- Method on type provides discoverable API
- Builder pattern for custom summary maintains immutability
- Edge case handling ensures robust behavior
- String formatting is straightforward in Rust

**Alternatives Considered**:
- Separate formatting function: Less discoverable
- Template-based formatting: Overkill for simple string formatting
- No custom summary support: Less flexible

---

### 6. Result Merging Implementation

**Question**: How to merge multiple batch results into a single aggregated result?

**Research Findings**:
- Need to combine statistics (counts, success rate)
- Need to merge error collections without duplicates
- Need to handle edge cases: empty results, all success, etc.
- Merged success rate should be recalculated from combined counts

**Decision**: Create `merge_results()` function that takes slice of `BatchResult`, combines all counts, merges error collections, and recalculates success rate from combined statistics. Preserve all errors from all batches.

**Rationale**:
- Standalone function provides flexibility for merging arbitrary batches
- Recalculating success rate from combined counts ensures accuracy
- Merging error collections preserves all error information
- Simple iteration and accumulation pattern

**Alternatives Considered**:
- Method on `BatchResult`: Less flexible, can only merge with one other result
- Weighted merging: Unnecessary complexity for simple aggregation
- Discard some errors: Breaks requirement for complete error preservation

---

## Summary

All technical decisions have been resolved. The implementation will:

1. Extend existing `BatchResult` type with convenience methods
2. Provide standalone aggregation utility functions
3. Support error filtering with separate filtered errors collection
4. Support opt-in error collection limits with truncation indication
5. Format errors with task identification included
6. Generate user-friendly summary messages
7. Merge multiple batch results accurately

No external dependencies or complex patterns required. Implementation uses standard Rust patterns and existing types.

