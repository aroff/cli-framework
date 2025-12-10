# Data Model: Batch Task Management

**Feature**: Batch Task Management  
**Date**: 2025-01-27

## Entities

### TaskDefinition

Represents a single task within a batch, containing the task logic and optional metadata.

**Fields**:
- `task: Box<dyn FnOnce() -> Pin<Box<dyn Future<Output = TaskResult> + Send>> + Send + 'static>` - The async task to execute
- `identifier: Option<String>` - Optional task identifier for error reporting (e.g., "file: image.jpg")
- `index: usize` - Positional index in the batch (0-based, assigned by framework)

**Relationships**:
- Belongs to one `TaskBatch`
- Produces one `TaskResult`

**Validation Rules**:
- Task must be `Send + 'static` for thread safety
- Identifier, if provided, should be non-empty (framework may validate or normalize)

**State Transitions**:
- Created → Spawned → Running → Completed (Success/Failure/Cancelled)

---

### TaskBatch

Represents a collection of tasks to be executed concurrently with shared concurrency limits.

**Fields**:
- `tasks: Vec<TaskDefinition>` - Collection of tasks to execute
- `concurrency_limit: Option<usize>` - Optional user-specified concurrency limit
- `default_limit: usize` - Framework-calculated default limit (CPU-based)
- `max_limit: usize` - Framework-enforced maximum limit (100)
- `effective_limit: usize` - Actual limit used (min of user limit, max limit, or default)

**Relationships**:
- Contains multiple `TaskDefinition`
- Produces one `BatchResult`

**Validation Rules**:
- Tasks collection must not be empty (empty batches handled separately)
- Concurrency limit must be > 0
- Effective limit = min(user_limit.unwrap_or(default_limit), max_limit)

**State Transitions**:
- Created → Spawning → Executing → Completed

---

### TaskResult

Represents the outcome of a single task execution.

**Fields**:
- `identifier: TaskIdentifier` - Task identifier (provided or positional index)
- `status: TaskStatus` - Execution status (Success, Failure, Cancelled)
- `value: Option<Box<dyn Any + Send>>` - Optional return value (for successful tasks)
- `error: Option<anyhow::Error>` - Error information (for failed tasks)

**Relationships**:
- Produced by one `TaskDefinition`
- Aggregated into `BatchResult`

**Validation Rules**:
- Status must match presence of value/error:
  - Success: value may be present, error must be None
  - Failure: error must be present, value must be None
  - Cancelled: both value and error must be None

**State Transitions**: N/A (result is final state)

---

### TaskStatus

Enum representing the execution status of a task.

**Variants**:
- `Success` - Task completed successfully
- `Failure` - Task failed with an error
- `Cancelled` - Task was cancelled before completion

**Relationships**: Used by `TaskResult`

---

### TaskIdentifier

Represents how a task is identified in results and errors.

**Variants**:
- `Provided(String)` - Application-provided identifier (e.g., "file: image.jpg")
- `Index(usize)` - Positional index in batch (0-based)

**Relationships**: Used by `TaskResult` and error messages

**Validation Rules**:
- Provided identifier should be non-empty (framework may normalize)
- Index must be valid (0 <= index < batch_size)

---

### BatchResult

Aggregated outcome of a batch operation, containing both statistics and individual results.

**Fields**:
- `total: usize` - Total number of tasks executed
- `successful: usize` - Number of successful tasks
- `failed: usize` - Number of failed tasks
- `cancelled: usize` - Number of cancelled tasks
- `success_rate: f64` - Success rate as percentage (successful / (successful + failed))
- `errors: Vec<(TaskIdentifier, anyhow::Error)>` - Collection of errors from failed tasks
- `results: Vec<TaskResult>` - Individual task results in completion order

**Relationships**:
- Produced by one `TaskBatch`
- Contains multiple `TaskResult`

**Validation Rules**:
- `total = successful + failed + cancelled`
- `success_rate = (successful as f64 / (successful + failed) as f64) * 100.0` (when successful + failed > 0)
- `success_rate = 0.0` when successful + failed == 0
- `errors.len() == failed`
- `results.len() == total`
- Results are in completion order (not spawn order)

**Computed Fields**:
- `all_succeeded: bool` - `failed == 0 && cancelled == 0`
- `has_failures: bool` - `failed > 0`
- `has_cancellations: bool` - `cancelled > 0`

---

## Type Definitions

```rust
// Task identifier (provided string or positional index)
pub enum TaskIdentifier {
    Provided(String),
    Index(usize),
}

// Task execution status
pub enum TaskStatus {
    Success,
    Failure,
    Cancelled,
}

// Individual task result
pub struct TaskResult {
    pub identifier: TaskIdentifier,
    pub status: TaskStatus,
    pub value: Option<Box<dyn Any + Send>>,  // For successful tasks
    pub error: Option<anyhow::Error>,         // For failed tasks
}

// Batch result with aggregated statistics
pub struct BatchResult {
    pub total: usize,
    pub successful: usize,
    pub failed: usize,
    pub cancelled: usize,
    pub success_rate: f64,
    pub errors: Vec<(TaskIdentifier, anyhow::Error)>,
    pub results: Vec<TaskResult>,
}

// Task definition for batch creation
pub struct TaskDefinition {
    task: Box<dyn FnOnce() -> Pin<Box<dyn Future<Output = TaskResult> + Send>> + Send + 'static>,
    identifier: Option<String>,
}
```

## Relationships Diagram

```
TaskBatch
  ├── contains: Vec<TaskDefinition>
  └── produces: BatchResult

TaskDefinition
  ├── belongs_to: TaskBatch
  └── produces: TaskResult

TaskResult
  ├── produced_by: TaskDefinition
  ├── contains: TaskIdentifier
  ├── contains: TaskStatus
  └── aggregated_into: BatchResult

BatchResult
  ├── produced_by: TaskBatch
  └── contains: Vec<TaskResult>
```

## State Transitions

### TaskBatch Lifecycle

```
Created
  └─> [spawn_batch() called]
      └─> Spawning
          └─> [all tasks spawned]
              └─> Executing
                  └─> [all tasks complete]
                      └─> Completed (BatchResult)
```

### TaskDefinition Lifecycle

```
Created
  └─> [spawned by batch]
      └─> Spawned
          └─> [permit acquired, task started]
              └─> Running
                  ├─> [task completes successfully]
                  │   └─> Completed (Success)
                  ├─> [task fails]
                  │   └─> Completed (Failure)
                  └─> [task cancelled]
                      └─> Completed (Cancelled)
```

## Notes

- All types must be `Send + Sync` for thread safety in async context
- `TaskResult::value` uses `Box<dyn Any>` to support generic return types
- Error aggregation preserves full error chains via `anyhow::Error`
- Results collection maintains completion order for deterministic behavior

