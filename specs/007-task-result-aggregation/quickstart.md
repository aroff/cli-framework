# Quickstart: Task Result Aggregation

**Feature**: Task Result Aggregation  
**Date**: 2025-01-27  
**Audience**: Developers using the CLI framework

## Overview

Task result aggregation utilities provide convenient methods for summarizing and formatting batch operation results. These utilities extend the existing `BatchResult` type with formatting methods and provide standalone functions for filtering, merging, and summarizing results.

## Basic Usage

### Generating Summary Messages

After a batch operation completes, generate a formatted summary:

```rust
use tui_framework::app::background_tasks::{BackgroundTaskManager, BatchResult};

// ... perform batch operation ...
let result: BatchResult = task_manager.spawn_batch(tasks, None).await?;

// Generate summary message
let summary = result.generate_summary();
println!("{}", summary);
// Output: "Completed 45 tasks: 42 successful, 3 failed (93.3% success rate)"
```

### Formatting Errors

Format errors with task identification for user-friendly display:

```rust
let result: BatchResult = task_manager.spawn_batch(tasks, None).await?;

if result.has_failures() {
    let error_output = result.format_errors();
    eprintln!("{}", error_output);
    // Output:
    // Errors (3):
    //   1. [file: image.jpg] Failed to process: permission denied
    //   2. [Task 5] Network error: connection timeout
    //   3. [file: data.json] Parse error: invalid JSON
}
```

### Custom Summary Messages

Override the auto-generated summary with a custom message:

```rust
let result: BatchResult = task_manager.spawn_batch(tasks, None).await?;

let custom_result = result.with_summary("Migration completed with some issues");
let summary = custom_result.generate_summary();
println!("{}", summary);
// Output: "Migration completed with some issues"
```

## Advanced Usage

### Error Filtering

Filter out certain errors from failure counts while preserving them for auditability:

```rust
use tui_framework::app::background_tasks::aggregation;

let results = vec![/* task results */];

// Filter out "not found" errors (treat as successful)
let aggregated = aggregation::aggregate_with_filter(results, |error| {
    !error.to_string().contains("not found")
});

// aggregated.failed only includes non-filtered errors
// aggregated.filtered_errors contains filtered errors
println!("Real failures: {}", aggregated.failed);
println!("Filtered errors: {}", aggregated.filtered_errors.len());
```

### Error Collection Limits

Limit error collection for memory efficiency with very large batches:

```rust
use tui_framework::app::background_tasks::aggregation;

let results = vec![/* 100,000+ task results */];

// Collect only first 1000 errors
let aggregated = aggregation::aggregate_with_limit(results, Some(1000));

if aggregated.truncated {
    println!("Warning: Error collection was truncated. {} errors collected.", 
             aggregated.errors.len());
}

// Statistics are always accurate regardless of truncation
println!("Success rate: {:.1}%", aggregated.success_rate);
```

### Merging Multiple Batch Results

Combine results from multiple batch operations:

```rust
use tui_framework::app::background_tasks::aggregation;

// Perform multiple batch operations
let result1 = task_manager.spawn_batch(batch1, None).await?;
let result2 = task_manager.spawn_batch(batch2, None).await?;
let result3 = task_manager.spawn_batch(batch3, None).await?;

// Merge all results
let merged = aggregation::merge_results(&[result1, result2, result3]);

println!("Total across all batches: {}", merged.total);
println!("Combined success rate: {:.1}%", merged.success_rate);
```

## Common Patterns

### Complete Reporting Workflow

```rust
use tui_framework::app::background_tasks::{BackgroundTaskManager, BatchResult};

async fn process_files(files: Vec<PathBuf>) -> anyhow::Result<()> {
    let mut task_manager = BackgroundTaskManager::new();
    
    // Spawn batch operations
    let tasks = files.into_iter().map(|file| {
        // ... create task definitions ...
    }).collect();
    
    let result = task_manager.spawn_batch(tasks, None).await?;
    
    // Generate and display summary
    println!("{}", result.generate_summary());
    
    // Display errors if any
    if result.has_failures() {
        eprintln!("\n{}", result.format_errors());
    }
    
    // Check overall success
    if !result.all_succeeded() {
        return Err(anyhow::anyhow!("Some files failed to process"));
    }
    
    Ok(())
}
```

### Migration Operations with Detailed Reporting

```rust
async fn run_migrations(migrations: Vec<Migration>) -> anyhow::Result<()> {
    let mut task_manager = BackgroundTaskManager::new();
    
    let tasks = migrations.into_iter().map(|migration| {
        // ... create migration task definitions ...
    }).collect();
    
    let result = task_manager.spawn_batch(tasks, None).await?;
    
    // Custom summary for migrations
    let result = result.with_summary("Database migration completed");
    
    println!("{}", result.generate_summary());
    println!("Success rate: {:.1}%", result.success_rate);
    
    if result.has_failures() {
        eprintln!("\nFailed migrations:\n{}", result.format_errors());
        return Err(anyhow::anyhow!("Migration completed with {} failures", result.failed));
    }
    
    Ok(())
}
```

### Bulk API Operations with Error Filtering

```rust
use tui_framework::app::background_tasks::aggregation;

async fn bulk_update(updates: Vec<Update>) -> anyhow::Result<()> {
    let mut task_manager = BackgroundTaskManager::new();
    
    let tasks = updates.into_iter().map(|update| {
        // ... create API update task definitions ...
    }).collect();
    
    let result = task_manager.spawn_batch(tasks, None).await?;
    
    // Filter out rate limit errors (treat as retryable, not failures)
    let filtered = aggregation::aggregate_with_filter(
        result.results().to_vec(),
        |error| !error.to_string().contains("rate limit")
    );
    
    println!("API operations: {} successful, {} failed", 
             filtered.successful, filtered.failed);
    
    if filtered.filtered_errors.len() > 0 {
        println!("Rate limited (retryable): {}", filtered.filtered_errors.len());
    }
    
    Ok(())
}
```

## Best Practices

1. **Always Check Results**: Use `all_succeeded()` or `has_failures()` before accessing error collections
2. **Display Summaries**: Always show summary messages to users for batch operations
3. **Format Errors**: Use `format_errors()` for user-friendly error display
4. **Use Limits Sparingly**: Only apply error collection limits when memory is a concern (100,000+ tasks)
5. **Preserve Auditability**: When filtering errors, remember they're preserved in `filtered_errors` collection
6. **Custom Summaries**: Use custom summaries for domain-specific messaging (e.g., "Migration completed")

## Performance Considerations

- Summary generation: < 10ms for batches up to 10,000 tasks
- Error formatting: Linear time, efficient for typical batch sizes
- Merging: Efficient for up to 100 batch results
- Error limits: Use only when necessary (100,000+ tasks) to avoid memory issues

## See Also

- [Batch Task Management Quickstart](../003-batch-task-management/quickstart.md) - For batch operation setup
- [API Contracts](./contracts/api.md) - For detailed API documentation
- [Data Model](./data-model.md) - For entity definitions and relationships

