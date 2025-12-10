# Data Model: Progress Reporting

**Date**: 2025-01-27  
**Feature**: 004-progress-reporting

## Entities

### ProgressReporter

Represents the current state of a long-running operation's progress.

**Fields**:
- `current: usize` - Current item number (1-indexed, but can be 0-indexed in practice)
- `total: Option<usize>` - Total number of items (None for indeterminate progress, also called "unknown total")
- `message: Option<String>` - Optional contextual message describing current operation

**Derived Fields**:
- `percentage: f64` - Calculated completion percentage (0.0 to 100.0, or 0.0 if total is None/indeterminate)
- `is_complete: bool` - True if current >= total (or false if total is None/indeterminate)

**Validation Rules**:
- `current` must be >= 0
- `total` must be > 0 if Some (cannot be zero)
- `percentage()` returns 0.0 if total is None or 0 (indeterminate progress)
- `percentage()` caps at 100.0 if current > total
- `is_complete()` returns false if total is None (indeterminate progress never "complete")

**State Transitions**:
- Created via `new(current: usize, total: usize)` or `with_message(current: usize, total: usize, message)`
  - Both constructors require `usize` for total (not `Option<usize>`)
  - For indeterminate progress (unknown total), applications set `total: None` in struct field after construction, or use a sentinel value
- Updated by creating new instances (immutable design)
- Progress moves forward: new instances should have current >= previous current

**Relationships**:
- Sent via `ProgressChannel` from background tasks to main application
- Consumed by CLI output formatting functions
- Displayed via terminal output

### ProgressChannel

Communication mechanism for progress updates (conceptual entity, implemented via `tokio::sync::mpsc`).

**Components**:
- `Sender<ProgressReporter>` - Cloned and passed to background tasks
- `Receiver<ProgressReporter>` - Held by main application for receiving updates

**Behavior**:
- Non-blocking: receiver uses `try_recv()` for non-blocking updates
- Best-effort: updates may be dropped if channel is full or receiver is slow
- Multiple senders: sender can be cloned for concurrent operations
- Single receiver: one receiver per progress channel

**Lifecycle**:
- Created when `spawn_with_progress()` is called
- Sender cloned and passed to background task
- Receiver returned to caller for polling
- Channel closed when sender is dropped (task completes or cancelled)

## Data Flow

```
Background Task
    │
    │ (cloned sender)
    ├─> ProgressReporter { current: 45, total: Some(200), message: Some("Processing file.jpg") }
    │
    └─> mpsc::Sender::send()
        │
        └─> ProgressChannel (mpsc::channel)
            │
            └─> mpsc::Receiver::try_recv() (non-blocking)
                │
                └─> Main Application Loop
                    │
                    ├─> format_progress_with_percentage()
                    │   └─> "[45/200] 22.5% - Processing file.jpg"
                    │
                    └─> print_progress_update()
                        └─> Terminal Output (in-place update with \r)
```

## Constraints

### Progress Update Constraints

1. **Progress Only Moves Forward**: New updates with current < last displayed current are ignored
2. **Best-Effort Delivery**: Progress updates may be dropped if channel is full or receiver is slow
3. **Non-Blocking**: Progress updates never block the operation or main event loop
4. **Optional Total**: Total can be None for indeterminate progress (graceful degradation)

### Formatting Constraints

1. **Percentage Capping**: Percentage display capped at 100.0% even if current > total
2. **Indeterminate Format**: When total is None, display count-only (no percentage)
3. **In-Place Updates**: Progress line overwrites current line using `\r` (carriage return)
4. **Final Newline**: Last progress update adds newline to preserve output

### Concurrent Operations Constraints

1. **Application Choice**: Display strategy (aggregated vs separate) chosen by application
2. **No Conflicts**: Multiple operations can send progress updates simultaneously without conflicts
3. **Channel Isolation**: Each operation gets its own progress channel (or shared if application chooses)

## Validation Examples

### Valid Progress Updates

```rust
// Standard progress
ProgressReporter::with_message(45, 200, "Processing file.jpg")
// → current: 45, total: Some(200), percentage: 22.5%

// Indeterminate progress (total unknown)
// Note: Constructors require usize, but struct field is Option<usize>
// Applications create with sentinel value or set total to None after construction
let mut progress = ProgressReporter::new(45, 0);  // Use 0 as sentinel
progress.total = None;  // Mark as indeterminate
// → current: 45, total: None, percentage: 0.0% (graceful degradation)

// Progress > 100%
ProgressReporter::new(200, 150)
// → current: 200, total: Some(150), percentage: 100.0% (capped)
```

### Invalid Progress Updates (Handled Gracefully)

```rust
// Zero total (handled as indeterminate)
ProgressReporter::new(0, 0)  // total: Some(0) → treated as None
// → percentage: 0.0%, is_complete: false

// Backwards progress (ignored)
// Last displayed: current = 50
// New update: current = 45
// → Update ignored, progress stays at 50
```

## Integration Points

### BackgroundTaskManager Integration

- New method: `spawn_with_progress<F>() -> (CancellationToken, Receiver<ProgressReporter>)`
- Extends existing `BackgroundTaskManager` struct
- Uses same cancellation token pattern as existing methods
- Progress channel separate from result and streaming channels

### CLI Output Integration

- New module: `src/cli_output.rs`
- Formatting functions consume `ProgressReporter`
- Display functions write to stdout (via `print!` and `println!`)
- Integrates with existing terminal output capabilities

### Runtime Integration

- Main event loop polls progress receiver (non-blocking)
- Progress updates formatted and displayed during render cycle
- No blocking operations, maintains 60 FPS target

