//! KeymapRegistry for managing keybindings

use crate::keymap::config::{AppCommand, KeyBinding, KeymapConfig};
use crossterm::event::KeyCode;
use std::collections::HashMap;

/// Registry for managing keybindings
pub struct KeymapRegistry {
    global: HashMap<KeyCode, KeyBinding>,
    per_view: HashMap<String, HashMap<KeyCode, KeyBinding>>,
}

impl KeymapRegistry {
    /// Create a new keymap registry
    pub fn new() -> Self {
        Self {
            global: HashMap::new(),
            per_view: HashMap::new(),
        }
    }

    /// Load configuration into registry
    pub fn load_config(&mut self, config: KeymapConfig) {
        for binding in config.global {
            self.global.insert(binding.key, binding);
        }
        for (view_id, bindings) in config.per_view {
            let view_map = self.per_view.entry(view_id).or_insert_with(HashMap::new);
            for binding in bindings {
                view_map.insert(binding.key, binding);
            }
        }
    }

    /// Get a global binding
    pub fn get_global(&self, key: KeyCode) -> Option<&KeyBinding> {
        self.global.get(&key)
    }

    /// Get a view-specific binding
    pub fn get_view_binding(&self, view_id: &str, key: KeyCode) -> Option<&KeyBinding> {
        self.per_view.get(view_id)?.get(&key)
    }
}

impl Default for KeymapRegistry {
    fn default() -> Self {
        Self::new()
    }
}
