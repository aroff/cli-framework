# Research: Progress Reporting for CLI Applications

**Date**: 2025-01-27  
**Feature**: 004-progress-reporting

## Technology Choices

### Decision: Use tokio::sync::mpsc for Progress Channels

**Rationale**: 
- Framework already uses `tokio::sync::mpsc` for result and streaming channels in `BackgroundTaskManager`
- Consistent with existing patterns reduces cognitive load and maintenance burden
- `mpsc` provides non-blocking `try_recv()` for progress updates in event loop
- Supports multiple senders (cloned) for concurrent operations
- No additional dependencies required

**Alternatives considered**:
- **tokio::sync::broadcast**: Overkill for point-to-point progress updates, adds complexity
- **tokio::sync::watch**: Single value updates, not suitable for accumulating progress
- **std::sync::mpsc**: Synchronous, would block async event loop
- **Custom channel implementation**: Unnecessary complexity, reinventing wheel

### Decision: In-Place Progress Updates with Carriage Return

**Rationale**:
- Standard CLI pattern for progress indicators (used by curl, wget, cargo, etc.)
- Prevents terminal clutter from hundreds of progress lines
- Uses ANSI escape sequence `\r` (carriage return) to overwrite current line
- Final update adds newline to preserve progress line in output
- Works across all platforms (Linux, macOS, Windows) via crossterm

**Alternatives considered**:
- **Multi-line progress**: Creates terminal clutter, harder to read
- **Progress bars (indicatif crate)**: Out of scope per spec (text-based only)
- **Separate progress window**: Overly complex for CLI framework

### Decision: Graceful Degradation for Indeterminate Progress

**Rationale**:
- Many real-world operations don't know total count upfront (streaming, dynamic lists)
- Better UX to show activity ("Processing item 45...") than nothing
- Maintains consistency with existing progress format (counts still shown)
- Simple implementation: check if total is None, format accordingly

**Alternatives considered**:
- **Reject operations without totals**: Too restrictive, breaks legitimate use cases
- **First-class indeterminate support**: Out of scope per spec, adds complexity
- **Estimate totals**: Unreliable, adds complexity without clear benefit

### Decision: Drop Older Updates When Display Can't Keep Up

**Rationale**:
- Progress updates are best-effort (per spec assumptions)
- Users care about current state, not historical updates
- Prevents buffer overflow and memory issues
- Ensures real-time feedback (no lag from buffering)
- Simple implementation: use `try_recv()` and only process latest

**Alternatives considered**:
- **Throttle updates**: Adds complexity, arbitrary rate limits
- **Buffer all updates**: Causes lag, memory issues with fast operations
- **Queue with backpressure**: Blocks operations, violates non-blocking requirement

### Decision: Progress Only Moves Forward (Ignore Backwards Updates)

**Rationale**:
- Prevents confusing UX where progress appears to decrease
- Out-of-order updates are edge case (shouldn't happen in normal operation)
- Simple validation: compare current count to last displayed value
- Maintains user confidence that operation is progressing

**Alternatives considered**:
- **Accept all updates**: Confusing UX, progress can go backwards
- **Reorder updates**: Complex buffering, adds latency
- **Error on backwards updates**: Too strict, breaks legitimate retry scenarios

### Decision: Cap Percentage at 100% When Count Exceeds Total

**Rationale**:
- Users expect progress percentages to not exceed 100%
- Actual counts still shown (transparency: "200/150 100%")
- Handles edge cases where operations process more items than expected
- Simple implementation: `min(percentage, 100.0)`

**Alternatives considered**:
- **Show actual percentage >100%**: Confusing, violates user expectations
- **Auto-update total**: Changes contract, may break applications
- **Error on >100%**: Too strict, breaks legitimate dynamic scenarios

### Decision: Application-Chosen Display Strategy for Concurrent Operations

**Rationale**:
- Different use cases need different display strategies
- Aggregated: Simple, clean for batch operations
- Separate lines: Useful for debugging, understanding individual operation progress
- Framework provides mechanism, application decides presentation
- Flexible design supports future enhancements

**Alternatives considered**:
- **Always aggregate**: Too restrictive, loses visibility into individual operations
- **Always separate**: Terminal clutter with many concurrent operations
- **Auto-detect strategy**: Complex heuristics, unpredictable behavior

## Integration Patterns

### Pattern: Extend BackgroundTaskManager with Progress Channel

**Approach**: Add new `spawn_with_progress()` method that:
1. Creates progress channel (mpsc::channel)
2. Clones sender for task
3. Returns cancellation token and progress receiver
4. Task sends progress updates via cloned sender
5. Main loop receives updates via receiver (non-blocking)

**Rationale**: 
- Consistent with existing `spawn()` and `spawn_streaming()` patterns
- Minimal API surface (one new method)
- Backward compatible (existing methods unchanged)
- Clear separation of concerns

### Pattern: Format Progress in CLI Output Module

**Approach**: Create new `src/cli_output.rs` module with:
- `format_progress()` - basic formatting
- `format_progress_with_percentage()` - with percentage
- `print_progress_update()` - in-place update with `\r`
- `print_progress_complete()` - final update with newline

**Rationale**:
- Centralizes formatting logic (DRY)
- Easy to test formatting functions independently
- Can be extended for future formatting options
- Separates formatting from progress data structure

## Performance Considerations

### Progress Update Latency

**Target**: <100ms from operation step completion to display (SC-001)

**Approach**:
- Use non-blocking `try_recv()` in event loop
- Drop older updates if display can't keep up
- Format progress synchronously (fast string operations)
- No async I/O for formatting (stdout flush is synchronous)

**Expected**: <1ms for formatting and display, well under 100ms target

### Performance Impact

**Target**: <5% degradation compared to operations without progress (SC-003)

**Approach**:
- Progress updates are non-blocking (best-effort)
- Channel operations are fast (in-memory)
- Formatting is lightweight (string operations)
- No additional I/O beyond existing stdout

**Expected**: <1% overhead for progress reporting, well under 5% target

## Error Handling

### Progress Channel Failures

**Approach**: 
- Progress updates are best-effort (per spec)
- If channel is full or closed, operation continues without progress updates
- No error propagation from progress updates to operation result
- Silent failure (operation succeeds even if progress updates fail)

**Rationale**: Progress is informational, not critical to operation success

### Edge Cases

**Division by zero (total = 0)**:
- Return 0.0% or handle gracefully
- Display as "0/0" or "0 items" without percentage

**Operation completes before progress updates**:
- Show final completion state
- Display "Completed" or final count/total

**Cancellation mid-execution**:
- Stop sending progress updates when cancelled
- Display final state before cancellation
- Handle via existing CancellationToken mechanism

## Testing Strategy

### Unit Tests
- `ProgressReporter::new()`, `with_message()`, `percentage()`, `is_complete()`
- Formatting functions with various inputs
- Edge cases: zero totals, >100%, missing totals

### Integration Tests
- End-to-end progress display in terminal
- Concurrent operations with progress
- Cancellation during progress reporting
- Performance benchmarks (latency, overhead)

### Contract Tests
- Progress reporting API contract
- Backward compatibility with existing BackgroundTaskManager
- Opt-in behavior (existing tasks unaffected)

