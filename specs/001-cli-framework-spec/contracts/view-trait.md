# View Trait Contract

**Purpose**: Defines the contract that all views must implement.

## Trait Definition

```rust
pub trait View {
    /// Stable identifier for this view. Literal, compile-time string.
    fn id(&self) -> &'static str;

    /// Name shown in the status bar / tabs.
    fn title(&self) -> &'static str;

    /// Called every frame to draw this view.
    fn render(&mut self, f: &mut Frame, area: Rect, ctx: &AppContext);

    /// Handles view-specific events (arrows, enter, letters, etc.).
    fn handle_event(&mut self, event: &Event, ctx: &mut AppContext) -> ViewResult;

    /// Help items specific to this view (used by '?').
    fn help_items(&self) -> Vec<HelpItem>;
}
```

## Contract Requirements

### id() -> &'static str

**Preconditions**: None  
**Postconditions**: 
- Returns a non-empty string literal
- Must be unique within the application
- Must be stable (same value for lifetime of view instance)

**Side Effects**: None

### title() -> &'static str

**Preconditions**: None  
**Postconditions**: 
- Returns a non-empty string literal
- Should be human-readable
- Used in status bar and help overlay

**Side Effects**: None

### render(&mut self, f: &mut Frame, area: Rect, ctx: &AppContext)

**Preconditions**: 
- `area` is a valid rectangle within terminal bounds
- `ctx` is initialized and valid

**Postconditions**:
- View is rendered to `f` within `area`
- Does not modify `ctx` (immutable reference)
- May modify internal view state (scroll position, etc.)

**Side Effects**: 
- Terminal output via `f`
- May update internal rendering state

**Error Handling**: Should not panic; handle errors gracefully

### handle_event(&mut self, event: &Event, ctx: &mut AppContext) -> ViewResult

**Preconditions**:
- `event` is a valid terminal event
- `ctx` is initialized and valid

**Postconditions**:
- Returns appropriate `ViewResult`:
  - `Handled` - Event was processed by view
  - `Ignored` - Event not relevant to this view
  - `SwitchView(view_id)` - Request to switch to another view
  - `ShowModal(message)` - Request to show modal with message
  - `Exit` - Request to exit application

**Side Effects**:
- May modify view state (selection, filters, etc.)
- May modify `ctx` (trigger operations, update state)
- May push `AppMessage` for user feedback

**Error Handling**: Should return `ViewResult::Handled` with error message rather than panicking

### help_items() -> Vec<HelpItem>

**Preconditions**: None  
**Postconditions**:
- Returns list of help items for this view
- Each item has key and description
- Should include view-specific keybindings

**Side Effects**: None

## Implementation Guidelines

1. **State Persistence**: Views are created once and persist state when switching. Implement `Default` or provide constructor that initializes state.

2. **Event Handling**: Views should handle events they care about and return `ViewResult::Ignored` for others.

3. **Rendering**: Views should handle terminal size gracefully. Framework provides `area`, but views should handle edge cases (very small areas).

4. **Help Items**: Should include all view-specific keybindings with clear descriptions.

5. **Error Handling**: Use `AppMessage` to communicate errors to users rather than panicking.

## Testing Contract

Contract tests should verify:
- `id()` returns unique, non-empty string
- `title()` returns non-empty string
- `render()` handles all valid `area` sizes without panicking
- `handle_event()` returns valid `ViewResult` for all event types
- `help_items()` returns non-empty list for views with keybindings

