//! ViewRegistry for managing registered views

use std::collections::HashMap;
use crate::view::View;

/// Registry for managing views
pub struct ViewRegistry {
    views: HashMap<String, Box<dyn View>>,
}

impl ViewRegistry {
    /// Create a new view registry
    pub fn new() -> Self {
        Self {
            views: HashMap::new(),
        }
    }

    /// Register a view
    pub fn register(&mut self, view: Box<dyn View>) {
        self.views.insert(view.id().to_string(), view);
    }

    /// Get a view by ID (returns a reference to the boxed view)
    pub fn get(&self, id: &str) -> Option<&Box<dyn View>> {
        self.views.get(id)
    }

    /// Get a mutable view by ID (returns a mutable reference to the boxed view)
    pub fn get_mut(&mut self, id: &str) -> Option<&mut Box<dyn View>> {
        self.views.get_mut(id)
    }

    /// Get iterator over view IDs
    pub fn views(&self) -> impl Iterator<Item = (&String, &Box<dyn View>)> {
        self.views.iter()
    }
}

impl Default for ViewRegistry {
    fn default() -> Self {
        Self::new()
    }
}

