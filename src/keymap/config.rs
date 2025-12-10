//! KeyBinding, KeymapConfig, and ViewSlot definitions
//!
//! Provides keybinding configuration for global and per-view bindings.

use crossterm::event::KeyCode;
use std::collections::HashMap;

/// View slot identifier (1-9)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ViewSlot {
    Slot1,
    Slot2,
    Slot3,
    Slot4,
    Slot5,
    Slot6,
    Slot7,
    Slot8,
    Slot9,
}

impl ViewSlot {
    /// Convert to KeyCode
    pub fn to_key_code(&self) -> KeyCode {
        match self {
            ViewSlot::Slot1 => KeyCode::Char('1'),
            ViewSlot::Slot2 => KeyCode::Char('2'),
            ViewSlot::Slot3 => KeyCode::Char('3'),
            ViewSlot::Slot4 => KeyCode::Char('4'),
            ViewSlot::Slot5 => KeyCode::Char('5'),
            ViewSlot::Slot6 => KeyCode::Char('6'),
            ViewSlot::Slot7 => KeyCode::Char('7'),
            ViewSlot::Slot8 => KeyCode::Char('8'),
            ViewSlot::Slot9 => KeyCode::Char('9'),
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
        self.per_view
            .entry(view_id)
            .or_insert_with(Vec::new)
            .push(binding);
        self
    }
}
