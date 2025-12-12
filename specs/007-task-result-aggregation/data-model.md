# Data Model: Task Result Aggregation

**Feature**: Task Result Aggregation  
**Date**: 2025-01-27  
**Phase**: 1 - Design & Contracts

## Entities

### 1. BatchResult (Extended)

Represents the aggregated outcome of batch task execution. This type already exists in `src/app/background_tasks.rs` from batch task management (003) and is extended with convenience methods.

**Fields** (existing):
- `total: usize` - Total number of tasks executed
- `successful: usize` - Number of successful tasks
- `failed: usize` - Number of failed tasks
- `cancelled: usize` - Number of cancelled tasks
- `success_rate: f64` - Success rate as percentage (successful / (successful + failed))
- `errors: Vec<(TaskIdentifier, anyhow::Error)>` - Collection of errors from failed tasks
- `results: Vec<BatchTaskResult>` - Individual task results in completion order

**New Fields** (for aggregation utilities):
- `filtered_errors: Vec<(TaskIdentifier, anyhow::Error)>` - Collection of errors excluded from failure count (when filtering is used)
- `truncated: bool` - Indicates if error collection was truncated due to limit (when limits are used)
- `custom_summary: Option<String>` - Optional custom summary message override

**Validation Rules**:
- `total = successful + failed + cancelled`
- `success_rate = (successful as f64 / (successful + failed) as f64) * 100.0` (when successful + failed > 0)
- `success_rate = 0.0` when successful + failed == 0
- `errors.len() == failed` (when no filtering applied)
- `filtered_errors.len() <= total` (when filtering applied)
- `results.len() == total`
- Results are in completion order (not spawn order)

**Relationships**:
- Contains multiple `BatchTaskResult` entities
- Contains multiple error tuples `(TaskIdentifier, anyhow::Error)`
- Produced by batch task management operations
- Used by aggregation utilities for formatting and merging

**State Transitions**: N/A (immutable data structure after creation)

**New Methods** (to be added):
- `generate_summary() -> String` - Generate formatted summary message
- `format_errors() -> String` - Format errors for display with task identification
- `with_summary(summary: impl Into<String>) -> Self` - Set custom summary message (builder pattern)
- `all_succeeded() -> bool` - Check if all tasks succeeded (already exists, verified)
- `has_failures() -> bool` - Check if any tasks failed (already exists, verified)
- `has_cancellations() -> bool` - Check if any tasks were cancelled (already exists, verified)

---

### 2. TaskIdentifier

Represents how a task is identified in results and errors. This type already exists from batch task management.

**Variants**:
- `Provided(String)` - Application-provided identifier (e.g., "file: image.jpg")
- `Index(usize)` - Positional index in batch (0-based)

**Relationships**: Used by `BatchResult` error collections

**Validation Rules**:
- Provided identifier should be non-empty (framework may normalize)
- Index must be valid (0 <= index < batch_size)

---

### 3. AggregationOptions

Configuration options for aggregation operations (new type for utility functions).

**Fields**:
- `max_errors: Option<usize>` - Optional limit on error collection (None = unlimited)
- `error_filter: Option<Box<dyn Fn(&anyhow::Error) -> bool>>` - Optional error filter predicate

**Validation Rules**:
- `max_errors` must be > 0 if `Some`
- Filter predicate must be provided if filtering is desired

**Relationships**: Used by aggregation utility functions

**State Transitions**: N/A (configuration struct)

---

## Type Definitions

```rust
// Task identifier (from batch task management)
pub enum TaskIdentifier {
    Provided(String),
    Index(usize),
}

// Extended BatchResult with aggregation utilities
pub struct BatchResult {
    pub total: usize,
    pub successful: usize,
    pub failed: usize,
    pub cancelled: usize,
    pub success_rate: f64,
    pub errors: Vec<(TaskIdentifier, anyhow::Error)>,
    pub results: Vec<BatchTaskResult>,
    // New fields for aggregation utilities:
    pub filtered_errors: Vec<(TaskIdentifier, anyhow::Error)>,
    pub truncated: bool,
    pub custom_summary: Option<String>,
}

// Aggregation options for utility functions
pub struct AggregationOptions {
    pub max_errors: Option<usize>,
    pub error_filter: Option<Box<dyn Fn(&anyhow::Error) -> bool>>,
}
```

---

## Relationships Diagram

```
BatchResult
  ├── contains: Vec<BatchTaskResult>
  ├── contains: Vec<(TaskIdentifier, anyhow::Error)> (errors)
  ├── contains: Vec<(TaskIdentifier, anyhow::Error)> (filtered_errors)
  └── used_by: Aggregation utilities

TaskIdentifier
  ├── used_by: BatchResult (in error collections)
  └── used_by: Error formatting

AggregationOptions
  └── used_by: Aggregation utility functions
```

---

## State Transitions

### BatchResult Lifecycle

```
Created (by batch task management)
  └─> [aggregation utilities applied]
      ├─> [generate_summary()] → String
      ├─> [format_errors()] → String
      ├─> [with_summary()] → BatchResult (immutable copy)
      └─> [merge_results()] → BatchResult (new merged result)
```

**Note**: `BatchResult` is immutable after creation. Methods like `with_summary()` return a new instance or use interior mutability patterns if needed.

---

## Validation Rules Summary

1. **Count Consistency**: `total = successful + failed + cancelled`
2. **Success Rate Calculation**: Based on `successful / (successful + failed)`, excluding cancelled tasks
3. **Error Collection**: `errors.len() == failed` when no filtering
4. **Filtered Errors**: `filtered_errors.len() <= total` when filtering applied
5. **Results Length**: `results.len() == total`
6. **Error Limit**: `errors.len() <= max_errors` when limit is set
7. **Truncation Indicator**: `truncated == true` when `errors.len() == max_errors` and more errors exist

---

## Edge Cases Handled

1. **Empty Results**: `total == 0` → all counts are 0, success_rate is 0.0
2. **All Success**: `failed == 0 && cancelled == 0` → success_rate is 100.0
3. **All Failure**: `successful == 0 && cancelled == 0` → success_rate is 0.0
4. **All Cancelled**: `successful == 0 && failed == 0` → success_rate is 0.0
5. **Zero Total**: Division by zero prevented in success rate calculation
6. **Error Truncation**: When limit reached, `truncated == true` indicates incomplete error collection
7. **Filtered Errors**: Filtered errors don't affect failure count or success rate

