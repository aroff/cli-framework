# Command Trait Contract (Async)

**Purpose**: Defines the contract for async command execution.

## Command Definition

```rust
pub struct Command {
    /// Unique command identifier
    pub id: CommandId,
    /// Short description (shown in command palette)
    pub summary: &'static str,
    /// Optional syntax hint (e.g., ":restart service=<name> env=<env>")
    pub syntax: Option<&'static str>,
    /// Optional category for grouping in palette
    pub category: Option<&'static str>,
    /// Async execution function
    pub execute: fn(
        &mut dyn AppContext,
        CommandArgs
    ) -> Pin<Box<dyn Future<Output = CommandResult> + Send>>,
}
```

## Contract Requirements

### id: CommandId

**Preconditions**: None  
**Postconditions**: Unique identifier for the command

**Constraints**: Must be unique within an application

### summary: &'static str

**Preconditions**: None  
**Postconditions**: Short description shown in command palette

### syntax: Option<&'static str>

**Preconditions**: None  
**Postconditions**: Optional syntax hint for command arguments

### category: Option<&'static str>

**Preconditions**: None  
**Postconditions**: Optional category for grouping in palette

### execute: fn(&mut dyn AppContext, CommandArgs) -> Pin<Box<dyn Future<Output = CommandResult> + Send>>

**Preconditions**:
- `ctx` is initialized and mutable
- `args` contains parsed command arguments
- `ctx` implements `Send + Sync`

**Postconditions**:
- Returns a future that resolves to `CommandResult`
- Future must be `Send` (can be moved between threads)
- Command execution may trigger async operations
- Loading indicator shown automatically during execution

**Side Effects**:
- May perform async operations (network, database, etc.)
- May update AppContext state
- May show messages to user via AppMessage

**Error Handling**:
- Should return `Err` with descriptive error message
- Framework will show error to user via status bar and modal
- Framework applies timeout (default 30s, configurable)

**Cancellation**:
- Operation may be cancelled if user cancels or view switches
- Implementation should handle cancellation gracefully

**Performance**:
- May take time for async operations
- UI remains responsive during execution

## Implementation Patterns

### Simple Async Command

```rust
Command {
    id: "refresh",
    summary: "Refresh all data sources",
    syntax: None,
    category: Some("data"),
    execute: |ctx, _args| {
        Box::pin(async move {
            let data_source = ctx.get_data_source();
            data_source.refresh(ctx).await?;
            ctx.set_status_message(AppMessage::info("Data refreshed"));
            Ok(())
        })
    },
}
```

### Command with Arguments

```rust
Command {
    id: "restart",
    summary: "Restart a service",
    syntax: Some(":restart service=<name> env=<env>"),
    category: Some("operations"),
    execute: |ctx, args| {
        Box::pin(async move {
            let service = args.named.get("service")
                .ok_or_else(|| anyhow!("service argument required"))?;
            let env = args.named.get("env")
                .ok_or_else(|| anyhow!("env argument required"))?;
            
            let client = ctx.get_service_client();
            client.restart_service(service, env).await?;
            
            ctx.set_status_message(
                AppMessage::info(format!("Restarted {} in {}", service, env))
            );
            Ok(())
        })
    },
}
```

## Testing Contract

Contract tests should verify:
- Command ID is unique
- Execution function is async and Send
- Execution handles arguments correctly
- Execution handles errors gracefully
- Execution can be cancelled
- Execution respects timeout

