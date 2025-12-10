# Quick Start: Batch Task Management

**Feature**: Batch Task Management  
**Date**: 2025-01-27

## Overview

The batch task management feature allows you to execute multiple background tasks concurrently with automatic concurrency control, result aggregation, and error handling.

## Basic Usage

### 1. Create Task Definitions

```rust
use tui_framework::app::BackgroundTaskManager;
use tui_framework::app::background_tasks::{task_definition, TaskResult};

// Define tasks with optional identifiers
let tasks = vec![
    task_definition(
        || async {
            // Your async task logic here
            process_file("file1.txt").await
        },
        Some("file1.txt".to_string()), // Optional identifier
    ),
    task_definition(
        || async { process_file("file2.txt").await },
        Some("file2.txt".to_string()),
    ),
];
```

### 2. Spawn Batch with Concurrency Limit

```rust
let mut manager = BackgroundTaskManager::new();

// Spawn batch with explicit concurrency limit
let result = manager.spawn_batch(tasks, Some(5)).await?;

// Or use default (CPU-based) limit
let result = manager.spawn_batch(tasks, None).await?;
```

### 3. Check Results

```rust
// Check aggregated statistics
println!("Total: {}, Success: {}, Failed: {}, Cancelled: {}", 
    result.total, 
    result.successful, 
    result.failed, 
    result.cancelled);
println!("Success rate: {:.2}%", result.success_rate);

// Check if all succeeded
if result.all_succeeded() {
    println!("All tasks completed successfully!");
} else {
    println!("Some tasks failed or were cancelled");
}
```

### 4. Access Individual Results

```rust
// Iterate over individual task results
for task_result in result.results() {
    match task_result.status {
        TaskStatus::Success => {
            println!("✓ {} succeeded", task_result.identifier.display());
        }
        TaskStatus::Failure => {
            println!("✗ {} failed: {}", 
                task_result.identifier.display(),
                task_result.error().unwrap());
        }
        TaskStatus::Cancelled => {
            println!("⊘ {} was cancelled", task_result.identifier.display());
        }
    }
}
```

### 5. Handle Errors

```rust
// Access errors from failed tasks
for (identifier, error) in result.errors() {
    eprintln!("Task {} failed: {}", identifier.display(), error);
    // Log, retry, or handle as needed
}
```

## Common Patterns

### Parallel File Processing

```rust
let files = vec!["file1.txt", "file2.txt", "file3.txt"];

let tasks: Vec<_> = files.into_iter()
    .map(|file| {
        let file = file.to_string();
        task_definition(
            move || {
                let file = file.clone();
                async move {
                    process_file(&file).await
                }
            },
            Some(file.clone()),
        )
    })
    .collect();

let result = manager.spawn_batch(tasks, Some(5)).await?;
```

### Batch API Operations

```rust
let records = vec![1, 2, 3, 4, 5];

let tasks: Vec<_> = records.into_iter()
    .map(|id| {
        task_definition(
            move || {
                let id = id;
                async move {
                    update_record(id).await
                }
            },
            Some(format!("record-{}", id)),
        )
    })
    .collect();

// Limit to 10 concurrent requests to respect API rate limits
let result = manager.spawn_batch(tasks, Some(10)).await?;
```

### Concurrent Data Processing

```rust
let chunks: Vec<Vec<Data>> = split_dataset(data, 100);

let tasks: Vec<_> = chunks.into_iter()
    .enumerate()
    .map(|(idx, chunk)| {
        task_definition(
            move || {
                let chunk = chunk;
                async move {
                    process_chunk(chunk).await
                }
            },
            Some(format!("chunk-{}", idx)),
        )
    })
    .collect();

let result = manager.spawn_batch(tasks, Some(20)).await?;
```

## Concurrency Limits

### Default Limit

If you don't specify a concurrency limit, the framework uses a CPU-based default:

```rust
// Uses default: CPU cores * 2
let result = manager.spawn_batch(tasks, None).await?;
```

### Custom Limit

Specify your own limit (up to maximum of 100):

```rust
// Limit to 5 concurrent tasks
let result = manager.spawn_batch(tasks, Some(5)).await?;

// Framework enforces maximum of 100
// Requesting 200 will be capped at 100
let result = manager.spawn_batch(tasks, Some(200)).await?;
```

### Sequential Execution

Set limit to 1 for sequential execution:

```rust
// Tasks run one at a time
let result = manager.spawn_batch(tasks, Some(1)).await?;
```

## Cancellation

### Cancel All Tasks in Batch

```rust
let batch_token = manager.spawn_batch(tasks, None).await?;

// Later, cancel the batch
manager.cancel_batch(&batch_token);
```

### Cancel Individual Tasks

Individual task cancellation is handled through the existing `cancel_task()` method using the task's cancellation token.

## Waiting for All Active Tasks

Wait for all tasks (from batches and individual spawns):

```rust
let result = manager.wait_for_all().await;

// Results include all active tasks, in completion order
println!("All tasks completed: {} total", result.total);
```

## Error Handling

### Task-Level Errors

Tasks that fail are included in the failed count:

```rust
let result = manager.spawn_batch(tasks, None).await?;

if result.has_failures() {
    for (identifier, error) in result.errors() {
        eprintln!("Task {} failed: {}", identifier.display(), error);
    }
}
```

### Timeout Handling

Tasks that exceed their timeout are treated as failures:

```rust
// Timeout is handled at the task level
// Timed-out tasks appear in result.failed and result.errors
```

## Best Practices

1. **Use meaningful identifiers**: Provide task identifiers for better error messages
   ```rust
   task_definition(task, Some("file: image.jpg".to_string()))
   ```

2. **Choose appropriate concurrency limits**: 
   - I/O-bound tasks: Higher limits (10-50)
   - CPU-bound tasks: Lower limits (CPU cores)
   - API calls: Respect rate limits (5-20)

3. **Handle errors appropriately**: Check `result.has_failures()` and iterate over `result.errors()`

4. **Monitor success rate**: Use `result.success_rate` to track batch health

5. **Use completion order**: Results are in completion order, not spawn order - useful for progress tracking

## Integration with Existing Code

Batch task management is fully backward compatible:

```rust
// Existing single-task spawning still works
let token = manager.spawn(async {
    // Single task
}).await;

// New batch spawning works alongside
let batch_result = manager.spawn_batch(tasks, None).await?;

// Both can be used together
let all_results = manager.wait_for_all().await;
```

## Next Steps

- See [API Contracts](./contracts/api.md) for detailed API documentation
- See [Data Model](./data-model.md) for type definitions
- See [Research](./research.md) for implementation details

