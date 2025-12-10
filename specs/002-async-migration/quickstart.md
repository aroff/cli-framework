# Quick Start: Async Migration Guide

**Feature**: 002-async-migration  
**Date**: 2025-12-09

## Overview

This guide helps you migrate your cli-framework application from synchronous (v0.1.0) to asynchronous (v0.2.0) runtime.

## Step 1: Update Dependencies

**Before** (v0.1.0):
```toml
[dependencies]
tui-framework = "0.1.0"
```

**After** (v0.2.0):
```toml
[dependencies]
tui-framework = "0.2.0"
tokio = { version = "1.0", features = ["full"] }
```

## Step 2: Update Main Function

**Before** (v0.1.0):
```rust
fn main() -> anyhow::Result<()> {
    let mut app = builder.build(MyContext)?;
    app.run()?;
    Ok(())
}
```

**After** (v0.2.0):
```rust
#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let mut app = builder.build(MyContext)?;
    app.run().await?;
    Ok(())
}
```

## Step 3: Update DataSource Implementation

**Before** (v0.1.0):
```rust
impl DataSource for MyDataSource {
    fn refresh(&mut self, ctx: &dyn AppContext) -> Result<()> {
        // Blocking operation
        let data = ctx.service().fetch_data()?;
        self.cache = data;
        Ok(())
    }
}
```

**After** (v0.2.0):
```rust
use async_trait::async_trait;

#[async_trait]
impl DataSource for MyDataSource {
    async fn refresh(&mut self, ctx: &dyn AppContext) -> Result<()> {
        // Async operation
        let data = ctx.service().fetch_data().await?;
        self.cache = data;
        Ok(())
    }
}
```

## Step 4: Update View Implementation

**Before** (v0.1.0):
```rust
impl View for MyView {
    fn handle_event(
        &mut self,
        event: &Event,
        ctx: &mut dyn AppContext
    ) -> ViewResult {
        // Sync event handling
        ViewResult::Handled
    }
}
```

**After** (v0.2.0):
```rust
use async_trait::async_trait;

#[async_trait]
impl View: Send + Sync for MyView {
    async fn handle_event(
        &mut self,
        event: &Event,
        ctx: &mut dyn AppContext
    ) -> ViewResult {
        // Can now use .await for async operations
        if let Event::Key(key) = event {
            if key.code == KeyCode::Char('r') {
                let data_source = ctx.get_data_source();
                data_source.refresh(ctx).await?;
            }
        }
        ViewResult::Handled
    }
}
```

## Step 5: Update Command Implementation

**Before** (v0.1.0):
```rust
Command {
    id: "refresh",
    summary: "Refresh data",
    execute: |ctx, _args| {
        ctx.service().refresh()?;
        Ok(())
    },
}
```

**After** (v0.2.0):
```rust
Command {
    id: "refresh",
    summary: "Refresh data",
    execute: |ctx, _args| {
        Box::pin(async move {
            ctx.service().refresh().await?;
            Ok(())
        })
    },
}
```

## Step 6: Update AppContext

**Before** (v0.1.0):
```rust
struct MyContext {
    service: MyServiceClient,
}

impl AppContext for MyContext {}
```

**After** (v0.2.0):
```rust
use std::sync::Arc;

struct MyContext {
    service: Arc<MyServiceClient>,  // Use Arc for Send + Sync
}

impl AppContext for MyContext {}
// AppContext now requires Send + Sync (compile-time enforced)
```

**Note**: If your context uses `Rc`, change to `Arc`. If you have non-Sync data, use `Arc<Mutex<T>>` or `Arc<RwLock<T>>`.

## Step 7: Use Async Service Clients

**Before** (v0.1.0):
```rust
// Had to use blocking bridges
let data = tokio::runtime::Handle::current()
    .block_on(client.fetch_data())?;
```

**After** (v0.2.0):
```rust
// Direct async usage
let data = client.fetch_data().await?;
```

## Key Changes Summary

1. ✅ Add `tokio` dependency
2. ✅ Make `main()` async with `#[tokio::main]`
3. ✅ Add `#[async_trait]` to DataSource, View implementations
4. ✅ Change `refresh()` and `handle_event()` to `async fn`
5. ✅ Update Command `execute` to return `Pin<Box<dyn Future + Send>>`
6. ✅ Ensure AppContext is `Send + Sync` (use `Arc` instead of `Rc`)
7. ✅ Use `.await` directly instead of `block_on()`

## What You Get

- ✅ Direct async service integration (no blocking bridges)
- ✅ Responsive UI during network operations
- ✅ Automatic loading indicators
- ✅ Concurrent operations support
- ✅ Background task system
- ✅ Operation cancellation on view switch

## Migration Checklist

- [ ] Update `Cargo.toml` dependencies
- [ ] Add `#[tokio::main]` to main function
- [ ] Update all `DataSource` implementations to async
- [ ] Update all `View` implementations to async
- [ ] Update all `Command` implementations to async
- [ ] Ensure `AppContext` is `Send + Sync`
- [ ] Replace `block_on()` calls with `.await`
- [ ] Update examples and tests
- [ ] Test async operations don't block UI
- [ ] Verify loading indicators appear

## Common Patterns

### Concurrent Data Fetching

```rust
let (servers, jobs, logs) = tokio::join!(
    server_source.refresh(ctx),
    job_source.refresh(ctx),
    log_source.refresh(ctx),
);
servers?;
jobs?;
logs?;
```

### Background Tasks

```rust
// Framework handles this automatically
// Just spawn async operations - framework manages them
let task = tokio::spawn(async move {
    long_operation().await
});
// Framework will handle result and update UI
```

### Error Handling

```rust
async fn refresh(&mut self, ctx: &dyn AppContext) -> Result<()> {
    let data = ctx.service().fetch().await
        .context("Failed to fetch data")?;
    Ok(())
}
```

## Need Help?

- See `examples/` directory for complete async examples
- Check `docs/async-migration.md` for detailed migration guide
- Review contract tests in `tests/contract/` for implementation patterns

