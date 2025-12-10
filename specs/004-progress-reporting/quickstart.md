# Quickstart: Progress Reporting

**Date**: 2025-01-27  
**Feature**: 004-progress-reporting

## Overview

Progress reporting enables CLI applications to provide real-time user feedback during long-running operations. Applications can report progress updates (current count, total count, optional messages) that are automatically formatted and displayed to users.

## Basic Usage

### Spawning a Task with Progress

```rust
use tui_framework::app::background_tasks::{BackgroundTaskManager, ProgressReporter};
use tui_framework::cli_output;

let mut manager = BackgroundTaskManager::new();

// Spawn task with progress reporting
let (token, mut progress_rx) = manager.spawn_with_progress(|progress_tx, cancel_token| {
    Box::pin(async move {
        let total = 100;
        
        for i in 1..=total {
            // Check for cancellation
            if cancel_token.is_cancelled() {
                break;
            }
            
            // Send progress update
            let progress = ProgressReporter::with_message(
                i,
                total,
                format!("Processing item {}", i)
            );
            let _ = progress_tx.send(progress).await;  // Best-effort, ignore errors
            
            // Simulate work
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        
        Ok(())
    })
});

// Poll progress updates (non-blocking, in your event loop)
while let Ok(progress) = progress_rx.try_recv() {
    cli_output::print_progress_update(&progress);
}

// When operation completes, show final progress
if let Ok(progress) = progress_rx.try_recv() {
    cli_output::print_progress_complete(&progress);
}
```

## Common Patterns

### Pattern 1: Simple Progress (Count Only)

```rust
let progress = ProgressReporter::new(45, 200);
cli_output::print_progress_update(&progress);
// Output: "[45/200] 22.5%"
```

### Pattern 2: Progress with Contextual Message

```rust
let progress = ProgressReporter::with_message(
    45,
    200,
    "Processing file.jpg"
);
cli_output::print_progress_update(&progress);
// Output: "[45/200] 22.5% - Processing file.jpg"
```

### Pattern 3: Indeterminate Progress (No Total)

```rust
// When total is unknown, gracefully degrade to count-only
let progress = ProgressReporter {
    current: 45,
    total: None,  // Indeterminate
    message: Some("Processing...".to_string()),
};
cli_output::print_progress_update(&progress);
// Output: "[45] Processing..." (no percentage)
```

### Pattern 4: Progress in Event Loop

```rust
// In your main event loop (e.g., App::run())
loop {
    // ... handle events ...
    
    // Poll progress updates (non-blocking)
    while let Ok(progress) = progress_rx.try_recv() {
        cli_output::print_progress_update(&progress);
    }
    
    // ... render frame ...
}
```

### Pattern 5: Multiple Concurrent Operations

The framework provides separate progress channels for each concurrent operation. Applications choose how to aggregate or display progress from multiple operations.

**Strategy 1: Separate Display (One Line Per Operation)**

```rust
let mut manager = BackgroundTaskManager::new();
let mut progress_receivers = Vec::new();

// Spawn multiple tasks with progress
for task_id in 0..5 {
    let (token, progress_rx) = manager.spawn_with_progress(|progress_tx, cancel_token| {
        let task_id = task_id;  // Capture task_id
        Box::pin(async move {
            for i in 1..=10 {
                if cancel_token.is_cancelled() {
                    break;
                }
                let progress = ProgressReporter::with_message(
                    i,
                    10,
                    format!("Task {}: item {}", task_id, i)
                );
                let _ = progress_tx.send(progress).await;
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
            Ok(())
        })
    });
    progress_receivers.push((task_id, progress_rx));
}

// Poll all progress receivers separately
for (task_id, progress_rx) in &mut progress_receivers {
    while let Ok(progress) = progress_rx.try_recv() {
        // Display each operation on separate line
        println!("Task {}: {}", task_id, cli_output::format_progress_with_percentage(&progress));
    }
}
```

**Strategy 2: Aggregated Display (Single Combined Line)**

```rust
// Aggregate progress from all operations
let mut total_current = 0;
let mut total_items = 0;
let mut active_tasks = progress_receivers.len();

// Poll all receivers and aggregate
for (_, progress_rx) in &mut progress_receivers {
    while let Ok(progress) = progress_rx.try_recv() {
        if let Some(total) = progress.total {
            total_current += progress.current;
            total_items += total;
        }
    }
}

// Display aggregated progress
if total_items > 0 {
    let aggregated = ProgressReporter::new(total_current, total_items);
    cli_output::print_progress_update(&aggregated);
}
```

**Note**: Aggregation logic is application-level. The framework provides the channels; applications choose how to combine and display progress from multiple operations.

## Integration with Existing Code

### Extending Background Tasks

Progress reporting extends the existing `BackgroundTaskManager` without breaking changes:

```rust
// Existing code still works
let token = manager.spawn(async {
    // ... existing task ...
    Ok(())
});

// New progress reporting (opt-in)
let (token, progress_rx) = manager.spawn_with_progress(|progress_tx, cancel_token| {
    Box::pin(async move {
        // ... task with progress ...
        Ok(())
    })
});
```

### Formatting Options

Choose the appropriate formatting function for your use case:

```rust
// Basic format (no percentage)
let formatted = cli_output::format_progress(&progress);
// "[45/200] Processing file.jpg"

// With percentage
let formatted = cli_output::format_progress_with_percentage(&progress);
// "[45/200] 22.5% - Processing file.jpg"

// In-place update (overwrites current line)
cli_output::print_progress_update(&progress);

// Final update (adds newline)
cli_output::print_progress_complete(&progress);
```

## Best Practices

### 1. Use Progress Updates Sparingly

Don't send progress updates for every tiny operation. Batch updates or send updates at meaningful milestones:

```rust
// Good: Update every 10 items
if i % 10 == 0 {
    let progress = ProgressReporter::new(i, total);
    let _ = progress_tx.send(progress).await;
}

// Avoid: Update every single item (too frequent)
for i in 1..=total {
    let progress = ProgressReporter::new(i, total);
    let _ = progress_tx.send(progress).await;  // Too many updates
}
```

### 2. Provide Meaningful Messages

Use contextual messages to help users understand what's happening:

```rust
// Good: Specific message
ProgressReporter::with_message(i, total, format!("Uploading {}", filename))

// Avoid: Generic message
ProgressReporter::with_message(i, total, "Processing".to_string())
```

### 3. Handle Cancellation

Always check for cancellation in long-running operations:

```rust
for i in 1..=total {
    if cancel_token.is_cancelled() {
        break;  // Exit early on cancellation
    }
    // ... send progress ...
}
```

### 4. Ignore Send Errors

Progress updates are best-effort. Don't let send failures affect your operation:

```rust
// Good: Ignore send errors
let _ = progress_tx.send(progress).await;

// Avoid: Propagating send errors
progress_tx.send(progress).await?;  // Don't do this
```

### 5. Poll Progress Non-Blocking

Use `try_recv()` in your event loop to avoid blocking. When updates arrive faster than display, only process the latest update:

```rust
// Good: Non-blocking poll, only process latest
let mut latest_progress = None;
while let Ok(progress) = progress_rx.try_recv() {
    latest_progress = Some(progress);  // Keep only latest
}
if let Some(progress) = latest_progress {
    cli_output::print_progress_update(&progress);
}

// Also good: Filter backwards updates
let mut last_displayed = 0;
while let Ok(progress) = progress_rx.try_recv() {
    if cli_output::should_display_progress(progress.current, last_displayed) {
        cli_output::print_progress_update(&progress);
        last_displayed = progress.current;
    }
}

// Avoid: Blocking receive
let progress = progress_rx.recv().await;  // Blocks event loop
```

## Edge Cases

### Progress > 100%

When current count exceeds total, percentage is capped at 100%:

```rust
let progress = ProgressReporter::new(200, 150);
assert_eq!(progress.percentage(), 100.0);  // Capped
cli_output::print_progress_update(&progress);
// Output: "[200/150] 100.0%" (shows actual counts)
```

### Indeterminate Progress

When total is unknown, gracefully degrade to count-only:

```rust
let progress = ProgressReporter {
    current: 45,
    total: None,
    message: Some("Processing...".to_string()),
};
cli_output::print_progress_update(&progress);
// Output: "[45] Processing..." (no percentage)
```

### Out-of-Order Updates

Updates with current < last displayed are ignored (progress only moves forward):

```rust
// Last displayed: current = 50
// New update: current = 45
// → Update ignored, progress stays at 50
```

### Fast Updates

If updates arrive faster than display, older updates are dropped:

```rust
// Multiple rapid updates
progress_tx.send(progress_1).await;  // May be dropped
progress_tx.send(progress_2).await;  // May be dropped
progress_tx.send(progress_3).await;  // Latest is displayed
```

## Testing

### Unit Tests

```rust
#[test]
fn test_progress_reporter() {
    let progress = ProgressReporter::new(45, 200);
    assert_eq!(progress.percentage(), 22.5);
    assert!(!progress.is_complete());
    
    let progress = ProgressReporter::new(200, 200);
    assert!(progress.is_complete());
}
```

### Integration Tests

```rust
#[tokio::test]
async fn test_progress_reporting() {
    let mut manager = BackgroundTaskManager::new();
    let (token, mut progress_rx) = manager.spawn_with_progress(|progress_tx, _| {
        Box::pin(async move {
            for i in 1..=10 {
                let progress = ProgressReporter::new(i, 10);
                let _ = progress_tx.send(progress).await;
            }
            Ok(())
        })
    });
    
    let mut updates = Vec::new();
    while let Ok(progress) = progress_rx.try_recv() {
        updates.push(progress);
    }
    
    assert!(!updates.is_empty());
}
```

## Next Steps

- See [data-model.md](./data-model.md) for detailed data model
- See [contracts/progress-api.md](./contracts/progress-api.md) for complete API reference
- See [research.md](./research.md) for design decisions and rationale

