# View Trait Contract (Async)

**Purpose**: Defines the contract for views in the async framework.

## Trait Definition

```rust
#[async_trait]
pub trait View: Send + Sync {
    /// Stable identifier for this view. Literal, compile-time string.
    fn id(&self) -> &'static str;

    /// Name shown in the status bar / tabs.
    fn title(&self) -> &'static str;

    /// Called every frame to draw this view.
    fn render(&mut self, f: &mut Frame, area: Rect, ctx: &dyn AppContext);

    /// Handles view-specific events (arrows, enter, letters, etc.).
    async fn handle_event(
        &mut self,
        event: &Event,
        ctx: &mut dyn AppContext
    ) -> ViewResult;

    /// Help items specific to this view (used by '?').
    fn help_items(&self) -> Vec<HelpItem>;

    /// Optional contextual information for header display (left side).
    fn header_info(&self) -> Option<Vec<(String, String)>> {
        None
    }

    /// Optional short help items for header display (right side).
    fn header_help(&self) -> Option<Vec<HelpItem>> {
        None
    }
}
```

## Contract Requirements

### fn id(&self) -> &'static str

**Preconditions**: None  
**Postconditions**: Returns a compile-time string literal that uniquely identifies the view

**Constraints**: Must be unique within an application

### fn title(&self) -> &'static str

**Preconditions**: None  
**Postconditions**: Returns human-readable name for the view

### fn render(&mut self, f: &mut Frame, area: Rect, ctx: &dyn AppContext)

**Preconditions**:
- `f` is a valid Frame
- `area` is within terminal bounds
- `ctx` is initialized

**Postconditions**:
- View is rendered to the frame
- Rendering is synchronous (called from async context but rendering itself is sync)

**Side Effects**: Draws to terminal buffer

**Performance**: Should complete quickly (rendering is sync, no async operations)

### async fn handle_event(&mut self, event: &Event, ctx: &mut dyn AppContext) -> ViewResult

**Preconditions**:
- `event` is a valid terminal event
- `ctx` is initialized and mutable
- View is currently active

**Postconditions**:
- Returns `ViewResult` indicating how event was handled
- May trigger async operations (network, database, etc.)
- May update view state or AppContext

**Side Effects**:
- May trigger async operations (DataSource refresh, service calls)
- May update internal view state
- May update AppContext
- Loading indicator shown automatically for async operations

**Error Handling**:
- Should return `ViewResult::ShowModal(AppMessage::error(...))` on errors
- Should not panic

**Cancellation**:
- Async operations may be cancelled if view switches
- Implementation should handle cancellation gracefully

**Performance**:
- Should return quickly for sync operations
- Async operations may take time but UI remains responsive

### fn help_items(&self) -> Vec<HelpItem>

**Preconditions**: None  
**Postconditions**: Returns help items for this view

### fn header_info(&self) -> Option<Vec<(String, String)>>

**Preconditions**: None  
**Postconditions**: Returns optional contextual information for header

### fn header_help(&self) -> Option<Vec<HelpItem>>

**Preconditions**: None  
**Postconditions**: Returns optional help items for header (max 5)

## Implementation Patterns

### Simple View

```rust
struct MyView {
    state: Arc<Mutex<ViewState>>,
}

#[async_trait]
impl View for MyView {
    fn id(&self) -> &'static str { "my.view" }
    fn title(&self) -> &'static str { "My View" }
    
    fn render(&mut self, f: &mut Frame, area: Rect, ctx: &dyn AppContext) {
        // Sync rendering
    }
    
    async fn handle_event(
        &mut self,
        event: &Event,
        ctx: &mut dyn AppContext
    ) -> ViewResult {
        match event {
            Event::Key(key) => {
                if key.code == KeyCode::Char('r') {
                    // Trigger async refresh
                    let data_source = ctx.get_data_source();
                    data_source.refresh(ctx).await?;
                    ViewResult::Handled
                } else {
                    ViewResult::Ignored
                }
            }
            _ => ViewResult::Ignored,
        }
    }
    
    fn help_items(&self) -> Vec<HelpItem> {
        vec![HelpItem::new("r", "Refresh data")]
    }
}
```

## Testing Contract

Contract tests should verify:
- `id()` returns unique identifier
- `title()` returns non-empty string
- `render()` draws view correctly
- `handle_event()` handles events appropriately
- `handle_event()` can perform async operations
- Implementation is `Send + Sync`
- Async operations can be cancelled

