# Research: Async Runtime Migration for CLI Framework

**Date**: 2025-12-09  
**Feature**: 002-async-migration

## Technology Choices

### Decision: Tokio as Async Runtime

**Rationale**: 
- Tokio is the de facto standard async runtime for Rust
- Widely adopted in the Rust ecosystem (reqwest, tokio-postgres, etc.)
- Mature, well-documented, actively maintained
- Provides all features needed: task spawning, timers, async I/O
- Framework will manage runtime internally (FR-018)

**Alternatives considered**:
- **async-std**: Less mature, smaller ecosystem
- **smol**: Lighter weight but less feature-complete
- **Manual futures**: Too low-level, would require significant framework code

### Decision: async-trait for Async Trait Support

**Rationale**:
- Rust doesn't natively support async in trait methods (until async fn in traits stabilizes)
- `async-trait` is the standard solution, widely used
- Provides clean API: `async fn method()` instead of returning `Pin<Box<dyn Future>>`
- Minimal performance overhead (boxing futures) acceptable for framework use case

**Alternatives considered**:
- **Manual Pin<Box<dyn Future>>**: More verbose, harder to use
- **Wait for native async traits**: Not stable yet, would block feature

### Decision: Keep Rendering Synchronous

**Rationale**:
- `ratatui` rendering is synchronous and fast
- No need to make rendering async (it's CPU-bound, not I/O-bound)
- Render from async context: `async fn run() { ... terminal.draw(|f| { render() })?; ... }`
- Maintains performance (SC-007: ≤16ms per frame)

**Alternatives considered**:
- **Async rendering**: Unnecessary complexity, ratatui doesn't support it
- **Separate render thread**: Over-engineering for TUI use case

### Decision: Background Task System

**Rationale**:
- Long-running operations need to update UI without blocking
- Tokio tasks + channels for communication
- Framework manages task lifecycle and cancellation
- Supports streaming data (FR-008)

**Pattern**:
```rust
// Spawn background task
let (tx, mut rx) = mpsc::channel();
tokio::spawn(async move {
    let result = long_operation().await;
    tx.send(result).await;
});

// In event loop: check for results
while let Ok(result) = rx.try_recv() {
    update_ui(result);
}
```

**Alternatives considered**:
- **Blocking operations**: Rejected - violates UI responsiveness requirement
- **Application-managed tasks**: Rejected - framework should provide this (FR-006)

### Decision: Terminal Event Reading with spawn_blocking

**Rationale**:
- `crossterm::event::read()` is blocking
- Use `tokio::task::spawn_blocking` to run in thread pool
- Prevents blocking the async runtime
- Maintains event loop responsiveness

**Pattern**:
```rust
async fn read_terminal_event() -> Result<Event> {
    tokio::task::spawn_blocking(|| {
        crossterm::event::read()
    }).await?
}
```

**Alternatives considered**:
- **Async terminal libraries**: None mature enough for production use
- **Polling with timeout**: Less efficient than spawn_blocking

### Decision: Send + Sync Bounds on AppContext

**Rationale**:
- Tokio tasks must be `Send` to move between threads
- Shared state must be `Sync` for concurrent access
- Required for thread safety in async context
- Applications must ensure their context types are Send + Sync

**Impact**:
- Applications using `Rc` or other non-Send types need to use `Arc`
- Applications with non-Sync data need synchronization primitives (Mutex, RwLock)

**Alternatives considered**:
- **Single-threaded runtime**: Would limit concurrency benefits
- **No bounds**: Unsafe, would cause runtime panics

### Decision: Automatic Loading Indicators

**Rationale**:
- Framework provides loading indicators automatically (FR-015)
- Shows spinner/indicator during async operations
- Appears within 100ms of operation start (SC-011)
- Improves user experience without application code

**Implementation**:
- Track active async operations
- Show loading indicator in status bar or view area
- Automatically hide when operation completes

**Alternatives considered**:
- **Application-provided**: Rejected - inconsistent UX, more work for developers
- **Optional**: Rejected - spec requires automatic (non-negotiable)

### Decision: Operation Cancellation on View Switch

**Rationale**:
- Operations cancelled when view switches (FR-016)
- Prevents stale results from updating wrong view
- Uses Tokio's cancellation tokens or task handles
- Configurable timeout (default 30s) for operations that should complete (FR-017)

**Pattern**:
```rust
let mut cancel_token = CancellationToken::new();
let task = tokio::spawn(async move {
    tokio::select! {
        _ = cancel_token.cancelled() => return,
        result = operation() => result,
    }
});

// On view switch:
cancel_token.cancel();
```

**Alternatives considered**:
- **Queue results**: More complex, may update wrong view
- **No cancellation**: Wastes resources, may cause confusion

## Integration Patterns

### Pattern: Async DataSource Implementation

```rust
#[async_trait]
impl DataSource for MyDataSource {
    async fn refresh(&mut self, ctx: &dyn AppContext) -> Result<()> {
        let data = ctx.service().fetch_data().await?;
        self.cache = data;
        Ok(())
    }
}
```

### Pattern: Async Command Execution

```rust
Command {
    execute: |ctx, args| {
        Box::pin(async move {
            ctx.service().do_work().await?;
            Ok(())
        })
    }
}
```

### Pattern: Concurrent Data Fetching

```rust
let (servers, jobs, logs) = tokio::join!(
    server_source.refresh(ctx),
    job_source.refresh(ctx),
    log_source.refresh(ctx),
);
```

## Best Practices

1. **Always use `#[async_trait]` for async trait methods** - Cleaner than manual futures
2. **Use `spawn_blocking` for blocking I/O** - Prevents blocking the runtime
3. **Use channels for background task communication** - Clean separation of concerns
4. **Implement cancellation tokens** - Allow operations to be cancelled gracefully
5. **Keep render calls synchronous** - Render from async context, but rendering itself is sync
6. **Use `Arc<Mutex<T>>` or `Arc<RwLock<T>>` for shared state** - Required for Send + Sync

## Dependencies

**New Dependencies**:
- `tokio = { version = "1.0", features = ["full"] }` - Async runtime
- `async-trait = "0.1"` - Async trait support
- `tokio-util = { version = "0.7", features = ["time"] }` - Async utilities

**Existing Dependencies** (no changes):
- `ratatui = "0.27"` - TUI rendering (remains sync)
- `crossterm = "0.28"` - Terminal I/O (has async support via spawn_blocking)
- `anyhow = "1.0"` - Error handling
- `serde = "1.0"` - Serialization

## Migration Strategy

1. **Add dependencies** to Cargo.toml
2. **Convert traits to async** using `#[async_trait]`
3. **Update event loop** to async with Tokio
4. **Add background task system**
5. **Update all implementations** to async
6. **Update examples** to use async patterns
7. **Add migration documentation**

## Testing Strategy

- **Unit tests**: Use `#[tokio::test]` for async tests
- **Integration tests**: Test async operations don't block UI
- **Contract tests**: Verify async trait implementations
- **Performance tests**: Verify latency targets (16ms frame time, 50ms interaction)

## Open Questions Resolved

All technical questions resolved:
- ✅ Tokio runtime management: Framework manages internally
- ✅ Async trait support: Use async-trait crate
- ✅ Terminal event reading: Use spawn_blocking
- ✅ Rendering: Keep sync, call from async context
- ✅ Loading indicators: Framework provides automatically
- ✅ Cancellation: Operations cancelled on view switch
- ✅ Timeouts: Configurable, default 30s

