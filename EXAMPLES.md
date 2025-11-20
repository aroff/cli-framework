# TUI Framework Examples

This document describes the three example applications included with the framework.

## Example 1: Simple

**Command**: `cargo run --example simple`

**Purpose**: Demonstrates the minimal setup required to create a working TUI.

**Features Shown**:
- Basic view registration
- GridView widget with DataSource
- Status bar (bottom of screen)
- Help overlay (press `?`)
- Basic navigation

**What You'll See**:
- A simple grid view displaying 3 items (Item 1, Item 2, Item 3)
- Status bar at the bottom
- Press `?` to see help overlay
- Press `q` to quit

**Code Highlights**:
- Minimal AppContext implementation
- Simple DataSource with in-memory data
- Basic View implementation

## Example 2: With Commands

**Command**: `cargo run --example with_commands`

**Purpose**: Demonstrates command execution, command palette, and multiple views.

**Features Shown**:
- Multiple views (Services view, Logs view)
- View headers with contextual information and keybindings
- Numeric key view switching (1 → Services, 2 → Logs)
- Command palette (press `:`)
- Command registration and execution
- Modal dialogs for command feedback
- Error handling demonstration

**What You'll See**:
- Services view showing 3 services (web-server, api-server, db-server)
- Logs view with placeholder content
- Press `:` to open command palette with 4 commands:
  - `restart` - Restart a service
  - `stop` - Stop a service
  - `info` - Show application information
  - `fail` - Demonstrate error handling (intentionally fails)
- Press 1/2 to switch between views
- Commands show success/error messages in status bar and modals

**Code Highlights**:
- Command registration
- Command execution with AppContext
- Multiple views with numeric key mapping
- Error handling with modals

## Example 3: Kitchen Sink

**Command**: `cargo run --example kitchen_sink`

**Purpose**: Comprehensive demonstration of all framework features.

**Features Shown**:
- View headers with dynamic contextual information
- GridView with interactive navigation (j/k keys)
- LogView with **live streaming logs** (updates every 500ms)
- Log filtering (press `/` in logs view)
- Follow mode toggle (press `f` in logs view)
- Scrolling controls (j/k, g/G, PageUp/PageDown)
- Command palette
- View switching
- All UI components working together

**What You'll See**:

### Resources View (1)
- Grid of 3 resources (web-server, api-server, db-server)
- Use `j`/`↓` or `k`/`↑` to navigate
- Selection highlighting

### Logs View (2)
- **Live streaming logs** - new log lines appear every 500ms
- Log levels: INFO, WARN, ERROR
- Timestamps on each line
- **Filtering**: Press `/` to enter filter mode, type keyword (e.g., "ERROR"), press Enter
- **Follow mode**: Press `f` to toggle auto-scroll to bottom
- **Scrolling**:
  - `j`/`↓`: Scroll down one line
  - `k`/`↑`: Scroll up one line
  - `g`: Jump to top
  - `G`: Jump to bottom
  - `PageUp`/`PageDown`: Page navigation
- When follow mode is ON, new logs automatically scroll into view
- When follow mode is OFF, you can scroll to review older logs

**Code Highlights**:
- SharedLogBuffer for thread-safe log streaming
- Background thread simulating log generation
- Interactive log filtering
- Follow mode implementation
- Complete integration of all framework features

## Running the Examples

All examples are interactive TUI applications. Here's how to use them:

1. **Start an example**: `cargo run --example <name>`
2. **Navigate**: Use the keys shown in help (press `?`)
3. **Quit**: Press `q`

### Common Controls (All Examples)
- `q`: Quit application
- `?`: Toggle help overlay
- `:`: Open command palette (if available)
- `Esc`: Close modals/overlays
- `1-9`: Switch views (if mapped)

### LogView Controls (Kitchen Sink)
- `/`: Enter filter mode (type keyword, press Enter)
- `f`: Toggle follow mode
- `j`/`↓`: Scroll down
- `k`/`↑`: Scroll up
- `g`: Scroll to top
- `G`: Scroll to bottom
- `PageUp`/`PageDown`: Page navigation

## Tips

- The examples will show startup messages in your terminal before entering TUI mode
- If the terminal is too small (< 80x24), the framework will gracefully degrade
- All examples demonstrate real framework features - the code can be used as reference
- The kitchen_sink example is the most comprehensive and shows all features working together

