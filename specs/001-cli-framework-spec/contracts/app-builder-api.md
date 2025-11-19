# AppBuilder API Contract

**Purpose**: Defines the public API for building and configuring TUI applications.

## Builder Pattern API

```rust
pub struct AppBuilder {
    // Internal state: registries, configuration
}

impl AppBuilder {
    pub fn new() -> Self;
    
    // UI Feature Toggles
    pub fn with_status_bar(mut self, enabled: bool) -> Self;
    pub fn with_help_overlay(mut self, enabled: bool) -> Self;
    pub fn with_command_palette(mut self, enabled: bool) -> Self;
    
    // View Registration
    pub fn register_view<V: View + 'static>(mut self, view: V) -> Self;
    pub fn map_view_slot(mut self, slot: ViewSlot, view_id: &'static str) -> Self;
    
    // Command Registration
    pub fn register_command(mut self, command: Command) -> Self;
    
    // Keybinding Configuration
    pub fn configure_keymap(mut self, keymap: KeymapConfig) -> Self;
    
    // Authentication (optional)
    pub fn with_authentication(mut self, auth_config: AuthConfig) -> Self;
    
    // Retry/Timeout Configuration
    pub fn with_retry_policy(mut self, policy: RetryPolicy) -> Self;
    
    // Observability (optional)
    pub fn with_observability(mut self, config: ObservabilityConfig) -> Self;
    
    // Build final App
    pub fn build(self, ctx: AppContext) -> Result<App, AppBuilderError>;
}
```

## Method Contracts

### new() -> Self

**Preconditions**: None  
**Postconditions**: 
- Returns new `AppBuilder` with default configuration
- Status bar, help overlay, command palette enabled by default
- Empty registries (no views, commands, keybindings)

**Side Effects**: None

### with_status_bar(enabled: bool) -> Self

**Preconditions**: None  
**Postconditions**:
- Status bar will be shown if `enabled == true`
- Status bar will be hidden if `enabled == false`

**Side Effects**: Updates internal configuration

### with_help_overlay(enabled: bool) -> Self

**Preconditions**: None  
**Postconditions**:
- Help overlay (`?` key) will be available if `enabled == true`
- Help overlay will be disabled if `enabled == false`

**Side Effects**: Updates internal configuration

### with_command_palette(enabled: bool) -> Self

**Preconditions**: None  
**Postconditions**:
- Command palette (`:` key) will be available if `enabled == true`
- Command palette will be disabled if `enabled == false`

**Side Effects**: Updates internal configuration

### register_view<V: View + 'static>(view: V) -> Self

**Preconditions**:
- `view.id()` must be unique (not already registered)
- `view` must be valid `View` implementation

**Postconditions**:
- View is registered and can be activated
- View ID is stored for later reference
- Returns `Self` for method chaining

**Side Effects**: 
- Adds view to internal registry
- May validate view (check for conflicts)

**Errors**: Returns error if view ID conflicts

### map_view_slot(slot: ViewSlot, view_id: &'static str) -> Self

**Preconditions**:
- `view_id` must be registered via `register_view()`
- `slot` must be valid `ViewSlot` (F1-F12)

**Postconditions**:
- Pressing the F-key for `slot` will switch to `view_id`
- Previous mapping for `slot` is replaced

**Side Effects**: Updates view slot mapping

**Errors**: Returns error if `view_id` not found

### register_command(command: Command) -> Self

**Preconditions**:
- `command.id` must be unique (not already registered)
- `command.execute` must be a valid function

**Postconditions**:
- Command is registered and available in command palette
- Command can be invoked via palette or keybinding

**Side Effects**: Adds command to internal registry

**Errors**: Returns error if command ID conflicts

### configure_keymap(keymap: KeymapConfig) -> Self

**Preconditions**:
- `keymap` must be valid configuration
- Keybindings must not conflict at same priority level

**Postconditions**:
- Global and per-view keybindings are configured
- Framework will resolve conflicts (view > global, modal > all)

**Side Effects**: Updates keymap registry

**Errors**: May warn about conflicts but will apply resolution rules

### with_authentication(auth_config: AuthConfig) -> Self

**Preconditions**:
- `auth_config` must be valid authentication configuration

**Postconditions**:
- Authentication is enabled for the application
- Login screen will be shown if required

**Side Effects**: Enables authentication subsystem

### with_retry_policy(policy: RetryPolicy) -> Self

**Preconditions**:
- `policy` must be valid retry configuration

**Postconditions**:
- Retry policies apply to all network operations
- Timeout handling is configured

**Side Effects**: Updates retry/timeout configuration

### with_observability(config: ObservabilityConfig) -> Self

**Preconditions**:
- `config` must be valid OpenTelemetry configuration

**Postconditions**:
- Observability is enabled (if configured)
- Framework operations emit logs/metrics/traces

**Side Effects**: Initializes observability subsystem

### build(ctx: AppContext) -> Result<App, AppBuilderError>

**Preconditions**:
- At least one view must be registered
- `ctx` must be initialized and valid
- All registered view IDs must be valid
- All mapped view slots must reference registered views

**Postconditions**:
- Returns `Ok(App)` if configuration is valid
- Returns `Err(AppBuilderError)` if validation fails
- `App` is ready to run

**Side Effects**:
- Validates entire configuration
- Initializes framework runtime
- Creates all views (once, at startup)
- Sets up event loop

**Errors**:
- `AppBuilderError::NoViews` - No views registered
- `AppBuilderError::InvalidViewId` - View slot references non-existent view
- `AppBuilderError::DuplicateViewId` - View ID conflicts
- `AppBuilderError::DuplicateCommandId` - Command ID conflicts
- `AppBuilderError::InvalidConfiguration` - Other configuration errors

## Usage Pattern

```rust
let app = AppBuilder::new()
    .with_status_bar(true)
    .with_help_overlay(true)
    .with_command_palette(true)
    .register_view(MyView::new())
    .register_view(AnotherView::new())
    .map_view_slot(ViewSlot::F1, "my.view.id")
    .map_view_slot(ViewSlot::F2, "another.view.id")
    .register_command(Command {
        id: "restart",
        summary: "Restart service",
        syntax: Some(":restart service=<name>"),
        execute: restart_command,
    })
    .with_retry_policy(RetryPolicy::default())
    .build(app_context)?;
```

## Testing Contract

Contract tests should verify:
- Builder methods can be chained
- Configuration is applied correctly
- Validation catches invalid configurations
- Error messages are descriptive
- Default configuration is sensible

