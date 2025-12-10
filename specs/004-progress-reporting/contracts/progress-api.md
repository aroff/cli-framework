# API Contract: Progress Reporting

**Date**: 2025-01-27  
**Feature**: 004-progress-reporting

## BackgroundTaskManager Extensions

### spawn_with_progress

Spawns a background task with progress reporting capability.

**Signature**:
```rust
pub fn spawn_with_progress<F>(
    &mut self,
    task: F,
) -> (CancellationToken, mpsc::Receiver<ProgressReporter>)
where
    F: FnOnce(
            mpsc::Sender<ProgressReporter>,
            CancellationToken,
        )
            -> std::pin::Pin<Box<dyn std::future::Future<Output = TaskResult> + Send>>
        + Send
        + 'static;
```

**Parameters**:
- `task: F` - Async task function that receives:
  - `mpsc::Sender<ProgressReporter>` - Cloned sender for sending progress updates
  - `CancellationToken` - Token for cancellation support

**Returns**:
- `CancellationToken` - Token for cancelling the task
- `mpsc::Receiver<ProgressReporter>` - Receiver for non-blocking progress updates

**Behavior**:
- Creates new progress channel (separate from result and streaming channels)
- Clones sender and passes to task
- Task can send progress updates via cloned sender
- Main application polls receiver using `try_recv()` (non-blocking)
- Progress updates are best-effort (may be dropped if channel is full)

**Errors**:
- None (progress updates are best-effort, don't affect operation result)

**Example**:
```rust
let mut manager = BackgroundTaskManager::new();

let (token, mut progress_rx) = manager.spawn_with_progress(|progress_tx, cancel_token| {
    Box::pin(async move {
        for i in 1..=10 {
            if cancel_token.is_cancelled() {
                break;
            }
            let progress = ProgressReporter::with_message(i, 10, format!("Item {}", i));
            let _ = progress_tx.send(progress).await;  // Best-effort, ignore errors
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        Ok(())
    })
});

// Poll progress updates (non-blocking)
while let Ok(progress) = progress_rx.try_recv() {
    cli_output::print_progress_update(&progress);
}
```

## ProgressReporter

### new

Creates a new progress reporter with current and total counts.

**Signature**:
```rust
pub fn new(current: usize, total: usize) -> Self
```

**Parameters**:
- `current: usize` - Current item number (0-indexed or 1-indexed, application choice)
- `total: usize` - Total number of items (must be > 0)

**Returns**: `ProgressReporter` with `message: None`

**Panics**: None

**Example**:
```rust
let progress = ProgressReporter::new(45, 200);
assert_eq!(progress.current, 45);
assert_eq!(progress.total, Some(200));
assert_eq!(progress.message, None);
```

### with_message

Creates a progress reporter with current, total, and contextual message.

**Signature**:
```rust
pub fn with_message(
    current: usize, 
    total: usize, 
    message: impl Into<String>
) -> Self
```

**Parameters**:
- `current: usize` - Current item number
- `total: usize` - Total number of items (must be > 0)
- `message: impl Into<String>` - Contextual message describing current operation

**Returns**: `ProgressReporter` with message set

**Panics**: None

**Example**:
```rust
let progress = ProgressReporter::with_message(45, 200, "Processing file.jpg");
assert_eq!(progress.message, Some("Processing file.jpg".to_string()));
```

### percentage

Calculates completion percentage.

**Signature**:
```rust
pub fn percentage(&self) -> f64
```

**Returns**: 
- `0.0` if `total` is `None` or `0`
- `(current as f64 / total as f64) * 100.0` if current <= total
- `100.0` if current > total (capped at 100%)

**Panics**: None

**Example**:
```rust
let progress = ProgressReporter::new(45, 200);
assert_eq!(progress.percentage(), 22.5);

let progress = ProgressReporter::new(200, 150);  // > 100%
assert_eq!(progress.percentage(), 100.0);  // Capped
```

### is_complete

Checks if progress is complete.

**Signature**:
```rust
pub fn is_complete(&self) -> bool
```

**Returns**:
- `true` if `current >= total` and `total` is `Some` and `> 0`
- `false` otherwise (including when `total` is `None`)

**Panics**: None

**Example**:
```rust
let progress = ProgressReporter::new(200, 200);
assert!(progress.is_complete());

let progress = ProgressReporter::new(45, 200);
assert!(!progress.is_complete());
```

## CLI Output Module

### format_progress

Formats progress for CLI output without percentage.

**Signature**:
```rust
pub fn format_progress(progress: &ProgressReporter) -> String
```

**Returns**: Formatted string like `"[45/200] Processing file.jpg"` or `"[45/200]"` if no message

**Example**:
```rust
let progress = ProgressReporter::with_message(45, 200, "Processing file.jpg");
assert_eq!(
    format_progress(&progress),
    "[45/200] Processing file.jpg"
);
```

### format_progress_with_percentage

Formats progress with percentage.

**Signature**:
```rust
pub fn format_progress_with_percentage(progress: &ProgressReporter) -> String
```

**Returns**: 
- Formatted string like `"[45/200] 22.5% - Processing file.jpg"` when total is available
- Formatted string like `"[45] Processing file.jpg"` when total is None (indeterminate)

**Example**:
```rust
let progress = ProgressReporter::with_message(45, 200, "Processing file.jpg");
assert_eq!(
    format_progress_with_percentage(&progress),
    "[45/200] 22.5% - Processing file.jpg"
);
```

### print_progress_update

Prints progress update in-place (overwrites current line).

**Signature**:
```rust
pub fn print_progress_update(progress: &ProgressReporter)
```

**Behavior**:
- Formats progress with percentage
- Prints with `\r` (carriage return) to overwrite current line
- Flushes stdout to ensure immediate display

**Example**:
```rust
print_progress_update(&progress);
// Terminal shows: "[45/200] 22.5% - Processing file.jpg" (overwrites previous line)
```

### print_progress_complete

Prints final progress line with newline.

**Signature**:
```rust
pub fn print_progress_complete(progress: &ProgressReporter)
```

**Behavior**:
- Formats progress with percentage
- Prints with `\r` and newline to preserve progress line
- Flushes stdout

**Example**:
```rust
print_progress_complete(&progress);
// Terminal shows: "[200/200] 100.0% - Complete\n" (newline added)
```

### should_display_progress

Helper function to filter backwards progress updates (progress only moves forward).

**Signature**:
```rust
pub fn should_display_progress(current: usize, last_displayed: usize) -> bool
```

**Parameters**:
- `current: usize` - Current item count from new progress update
- `last_displayed: usize` - Last displayed item count

**Returns**: 
- `true` if `current >= last_displayed` (progress moved forward or stayed same)
- `false` if `current < last_displayed` (backwards update, should be ignored)

**Behavior**:
- Filters out backwards progress updates to ensure progress only moves forward
- Applications should maintain `last_displayed` state and call this before displaying updates

**Example**:
```rust
let mut last_displayed = 0;

while let Ok(progress) = progress_rx.try_recv() {
    if should_display_progress(progress.current, last_displayed) {
        cli_output::print_progress_update(&progress);
        last_displayed = progress.current;
    }
    // Backwards updates are silently ignored
}
```

## Backward Compatibility

### Existing API Unchanged

- `BackgroundTaskManager::spawn()` - Unchanged
- `BackgroundTaskManager::spawn_streaming()` - Unchanged
- `BackgroundTaskManager::spawn_periodic()` - Unchanged
- All existing methods remain functional

### Opt-In Behavior

- Applications must explicitly call `spawn_with_progress()` to use progress reporting
- Existing tasks continue to work without progress reporting
- No breaking changes to existing API

## Error Handling Contract

### Progress Update Failures

- Progress updates are **best-effort**
- If `progress_tx.send()` fails (channel closed/full), operation continues
- Progress update failures do not affect operation result
- No error propagation from progress updates

### Formatting Failures

- Formatting functions never fail (pure string operations)
- Display functions may fail on stdout write, but errors are ignored (best-effort)

### Channel Failures

- If progress receiver is dropped, sender operations are no-ops
- If progress sender is dropped, receiver returns `Err(RecvError::Closed)`
- Applications should handle `try_recv()` errors gracefully (ignore or log)

