# API Contracts: Batch Task Management

**Feature**: Batch Task Management  
**Date**: 2025-01-27  
**Format**: Rust function signatures

## BackgroundTaskManager Extensions

### Batch Task Spawning

```rust
impl BackgroundTaskManager {
    /// Spawn a batch of tasks with optional concurrency limit
    ///
    /// # Arguments
    /// * `tasks` - Collection of task definitions with optional identifiers
    /// * `concurrency_limit` - Optional maximum concurrent tasks (defaults to CPU-based limit, max 100)
    ///
    /// # Returns
    /// `BatchResult` containing aggregated statistics and individual task results
    ///
    /// # Errors
    /// Returns error if batch creation fails (e.g., invalid concurrency limit)
    pub async fn spawn_batch(
        &mut self,
        tasks: Vec<TaskDefinition>,
        concurrency_limit: Option<usize>,
    ) -> Result<BatchResult>;
}
```

**Contract Details**:
- Tasks are spawned concurrently (not sequentially)
- Framework waits for all tasks to complete before returning
- Empty task collections return empty `BatchResult` immediately
- Concurrency limit is enforced (default = CPU cores * 2, max = 100)
- Results are returned in completion order

---

### Task Definition Creation

```rust
/// Create a task definition for batch processing
///
/// # Arguments
/// * `task` - Async task function returning `TaskResult`
/// * `identifier` - Optional task identifier for error reporting
///
/// # Returns
/// `TaskDefinition` ready for batch execution
pub fn task_definition<F, Fut>(
    task: F,
    identifier: Option<String>,
) -> TaskDefinition
where
    F: FnOnce() -> Fut + Send + 'static,
    Fut: Future<Output = TaskResult> + Send + 'static;
```

**Contract Details**:
- Task must be `Send + 'static` for thread safety
- Identifier, if provided, is preserved in results and errors
- Positional index is assigned automatically by framework

---

### Wait for All Active Tasks

```rust
impl BackgroundTaskManager {
    /// Wait for all currently active tasks to complete
    ///
    /// This includes tasks spawned individually and tasks from batches.
    ///
    /// # Returns
    /// `BatchResult` containing results from all active tasks
    ///
    /// # Behavior
    /// - Blocks until all tasks finish
    /// - Returns immediately with empty results if no tasks are active
    /// - Results are in completion order (not spawn order)
    pub async fn wait_for_all(&mut self) -> BatchResult;
}
```

**Contract Details**:
- Waits for tasks regardless of how they were spawned
- Results in completion order
- Returns empty result if no active tasks

---

### Batch Cancellation

```rust
impl BackgroundTaskManager {
    /// Cancel all tasks in a batch
    ///
    /// # Arguments
    /// * `batch_token` - Cancellation token for the batch
    ///
    /// # Behavior
    /// - Cancels all tasks associated with the batch token
    /// - Other tasks (not in this batch) continue executing
    /// - Cancelled tasks are tracked separately in results
    pub fn cancel_batch(&mut self, batch_token: &CancellationToken);
}
```

**Contract Details**:
- Cancellation is graceful (tasks can clean up)
- Other tasks in batch continue if individual cancellation used
- Cancelled tasks counted separately from failures

---

## Type Contracts

### TaskResult

```rust
pub struct TaskResult {
    pub identifier: TaskIdentifier,
    pub status: TaskStatus,
    pub value: Option<Box<dyn Any + Send>>,
    pub error: Option<anyhow::Error>,
}

impl TaskResult {
    /// Check if task succeeded
    pub fn is_success(&self) -> bool;

    /// Check if task failed
    pub fn is_failure(&self) -> bool;

    /// Check if task was cancelled
    pub fn is_cancelled(&self) -> bool;

    /// Get error if task failed
    pub fn error(&self) -> Option<&anyhow::Error>;

    /// Get value if task succeeded (requires type downcast)
    pub fn value<T: 'static>(&self) -> Option<&T>;
}
```

### BatchResult

```rust
pub struct BatchResult {
    pub total: usize,
    pub successful: usize,
    pub failed: usize,
    pub cancelled: usize,
    pub success_rate: f64,
    pub errors: Vec<(TaskIdentifier, anyhow::Error)>,
    pub results: Vec<TaskResult>,
}

impl BatchResult {
    /// Check if all tasks succeeded
    pub fn all_succeeded(&self) -> bool;

    /// Check if any tasks failed
    pub fn has_failures(&self) -> bool;

    /// Check if any tasks were cancelled
    pub fn has_cancellations(&self) -> bool;

    /// Get errors from failed tasks
    pub fn errors(&self) -> &[(TaskIdentifier, anyhow::Error)];

    /// Get individual task results
    pub fn results(&self) -> &[TaskResult];
}
```

### TaskIdentifier

```rust
pub enum TaskIdentifier {
    Provided(String),
    Index(usize),
}

impl TaskIdentifier {
    /// Get display string for identifier
    pub fn display(&self) -> String;
}
```

### TaskStatus

```rust
pub enum TaskStatus {
    Success,
    Failure,
    Cancelled,
}
```

---

## Error Contracts

### BatchError

```rust
pub enum BatchError {
    /// Invalid concurrency limit (must be > 0 and <= max)
    InvalidConcurrencyLimit { requested: usize, max: usize },
    
    /// Batch creation failed
    CreationFailed { reason: String },
    
    /// Task execution error (wrapped from individual task)
    TaskError { identifier: TaskIdentifier, error: anyhow::Error },
}
```

---

## Usage Examples

### Basic Batch Execution

```rust
let mut manager = BackgroundTaskManager::new();

let tasks = vec![
    task_definition(|| async { process_file("file1.txt").await }, Some("file1.txt".to_string())),
    task_definition(|| async { process_file("file2.txt").await }, Some("file2.txt".to_string())),
];

let result = manager.spawn_batch(tasks, Some(5)).await?;

println!("Processed {} files: {} succeeded, {} failed", 
    result.total, result.successful, result.failed);
```

### Batch with Default Concurrency

```rust
let result = manager.spawn_batch(tasks, None).await?;
// Uses CPU-based default limit
```

### Accessing Individual Results

```rust
for task_result in result.results() {
    match task_result.status {
        TaskStatus::Success => {
            println!("Task {} succeeded", task_result.identifier.display());
        }
        TaskStatus::Failure => {
            println!("Task {} failed: {}", 
                task_result.identifier.display(),
                task_result.error().unwrap());
        }
        TaskStatus::Cancelled => {
            println!("Task {} was cancelled", task_result.identifier.display());
        }
    }
}
```

---

## Backward Compatibility

All existing `BackgroundTaskManager` methods remain unchanged:
- `spawn()` - Still works for individual tasks
- `spawn_streaming()` - Still works for streaming tasks
- `spawn_periodic()` - Still works for periodic tasks
- `cancel_all()` - Cancels all tasks (including batch tasks)
- `cancel_task()` - Cancels individual tasks

New batch methods are additive and do not modify existing behavior.

