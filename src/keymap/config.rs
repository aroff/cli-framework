//! KeyBinding, KeymapConfig, and ViewSlot definitions
//!
//! Provides keybinding configuration for global and per-view bindings.

use crossterm::event::KeyCode;
use std::collections::HashMap;

/// View slot identifier (F1-F12)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ViewSlot {
    F1,
    F2,
    F3,
    F4,
    F5,
    F6,
    F7,
    F8,
    F9,
    F10,
    F11,
    F12,
}

impl ViewSlot {
    /// Convert to KeyCode
    pub fn to_key_code(&self) -> KeyCode {
        match self {
            ViewSlot::F1 => KeyCode::F(1),
            ViewSlot::F2 => KeyCode::F(2),
            ViewSlot::F3 => KeyCode::F(3),
            ViewSlot::F4 => KeyCode::F(4),
            ViewSlot::F5 => KeyCode::F(5),
            ViewSlot::F6 => KeyCode::F(6),
            ViewSlot::F7 => KeyCode::F(7),
            ViewSlot::F8 => KeyCode::F(8),
            ViewSlot::F9 => KeyCode::F(9),
            ViewSlot::F10 => KeyCode::F(10),
            ViewSlot::F11 => KeyCode::F(11),
            ViewSlot::F12 => KeyCode::F(12),
        }
    }
}

/// Action that can be triggered by a keybinding
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AppCommand {
    /// Switch to a different view
    SwitchView(String),
    /// Invoke an action by ID
    InvokeAction(String),
    /// Run a command with arguments
    RunCommand(String, HashMap<String, String>),
}

/// Key binding mapping a key to an action
#[derive(Debug, Clone)]
pub struct KeyBinding {
    /// The keyboard key or key sequence
    pub key: KeyCode,
    /// The action to execute
    pub action: AppCommand,
}

impl KeyBinding {
    /// Create a new key binding
    pub fn new(key: KeyCode, action: AppCommand) -> Self {
        Self { key, action }
    }
}

/// Keymap configuration
///
/// Supports global bindings (apply to all views) and per-view bindings.
/// View-specific bindings override global bindings.
#[derive(Debug, Clone, Default)]
pub struct KeymapConfig {
    /// Global keybindings (apply to all views)
    pub global: Vec<KeyBinding>,
    /// Per-view keybindings (keyed by view ID)
    pub per_view: HashMap<String, Vec<KeyBinding>>,
}

impl KeymapConfig {
    /// Create a new empty keymap configuration
    pub fn new() -> Self {
        Self::default()
    }

    /// Add a global keybinding
    pub fn add_global(mut self, binding: KeyBinding) -> Self {
        self.global.push(binding);
        self
    }

    /// Add a per-view keybinding
    pub fn add_view_binding(mut self, view_id: String, binding: KeyBinding) -> Self {
        self.per_view.entry(view_id).or_insert_with(Vec::new).push(binding);
        self
    }
}

