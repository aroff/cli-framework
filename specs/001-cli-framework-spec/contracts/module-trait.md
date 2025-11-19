# Module Trait Contract

**Purpose**: Defines the contract for internal modularization, allowing applications to group related views, commands, and keybindings.

## Trait Definition

```rust
pub trait Module {
    /// Stable identifier for this module. Literal, compile-time string.
    fn id(&self) -> &'static str;

    /// Called during application build time to register module components.
    fn register(&self, builder: &mut AppBuilder) -> anyhow::Result<()>;
}
```

## Contract Requirements

### id() -> &'static str

**Preconditions**: None  
**Postconditions**:
- Returns a non-empty string literal
- Must be unique within the application
- Must be stable

**Side Effects**: None

### register(&self, builder: &mut AppBuilder) -> anyhow::Result<()>

**Preconditions**:
- `builder` is initialized and valid

**Postconditions**:
- On success: Module's views, commands, and keybindings are registered with the builder
- On error: Returns `Err` with descriptive error message (e.g. registration conflict)

**Side Effects**:
- Modifies `builder` state by adding components

## Usage Pattern

```rust
struct AirflowModule;

impl Module for AirflowModule {
    fn id(&self) -> &'static str {
        "airflow"
    }

    fn register(&self, builder: &mut AppBuilder) -> anyhow::Result<()> {
        builder
            .register_view(DagsView::new())
            .register_view(RunsView::new())
            .register_command(trigger_dag_command())
            // ... register other components
            ;
        Ok(())
    }
}

// In main:
let mut builder = AppBuilder::new();
AirflowModule.register(&mut builder)?;
// ...
```

