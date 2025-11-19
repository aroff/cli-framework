//! KeymapResolver for resolving keybinding conflicts
//!
//! Resolution priority: modals > view-specific > global

use crate::keymap::config::AppCommand;
use crate::keymap::registry::KeymapRegistry;
use crossterm::event::KeyCode;

/// Resolver for keybinding conflicts
pub struct KeymapResolver {
    registry: KeymapRegistry,
}

impl KeymapResolver {
    /// Create a new keymap resolver
    pub fn new(registry: KeymapRegistry) -> Self {
        Self { registry }
    }

    /// Resolve a key press (priority: modal > view-specific > global)
    pub fn resolve(&self, key: KeyCode, view_id: Option<&str>, modal_active: bool) -> Option<AppCommand> {
        // If modal is active, modals should handle all keys
        // For now, we'll let modals handle their own keys
        // and only check view/global if modal is not active
        if modal_active {
            return None; // Modal handles its own keys
        }

        // Check view-specific bindings first (they override global)
        if let Some(view_id) = view_id {
            if let Some(binding) = self.registry.get_view_binding(view_id, key) {
                return Some(binding.action.clone());
            }
        }

        // Check global bindings
        if let Some(binding) = self.registry.get_global(key) {
            return Some(binding.action.clone());
        }

        None
    }
}
