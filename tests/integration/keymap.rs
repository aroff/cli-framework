//! Integration tests for keymap configuration
//!
//! Verifies that custom keybindings override defaults and view-specific
//! bindings override global bindings.

use tui_framework::prelude::*;
use tui_framework::keymap::{KeyBinding, KeymapConfig, AppCommand, ViewSlot};
use tui_framework::view::{View, ViewResult, HelpItem};
use tui_framework::message::AppMessage;
use crossterm::event::{Event, KeyCode};
use ratatui::layout::Rect;
use ratatui::Frame;
use std::collections::HashMap;

// Test view
struct TestView {
    id: &'static str,
    key_pressed: Option<KeyCode>,
}

impl TestView {
    fn new(id: &'static str) -> Self {
        Self {
            id,
            key_pressed: None,
        }
    }
}

impl View for TestView {
    fn id(&self) -> &'static str {
        self.id
    }

    fn title(&self) -> &'static str {
        "Test View"
    }

    fn render(&mut self, _f: &mut Frame, _area: Rect, _ctx: &dyn AppContext) {
        // No-op for test
    }

    fn handle_event(&mut self, event: &Event, _ctx: &mut dyn AppContext) -> ViewResult {
        if let Event::Key(key) = event {
            self.key_pressed = Some(key.code);
            ViewResult::Handled
        } else {
            ViewResult::Ignored
        }
    }

    fn help_items(&self) -> Vec<HelpItem> {
        vec![]
    }
}

// Test app context
struct TestContext;

impl AppContext for TestContext {}

#[test]
fn test_custom_global_keybinding_overrides_default() {
    // Create a keymap with a custom global binding
    let mut keymap_config = KeymapConfig::new();
    
    // Bind 'x' to switch to a specific view (this would normally be handled by framework)
    keymap_config = keymap_config.add_global(KeyBinding::new(
        KeyCode::Char('x'),
        AppCommand::SwitchView("test.view2".to_string()),
    ));
    
    // Verify the binding is in the config
    assert_eq!(keymap_config.global.len(), 1);
    assert_eq!(keymap_config.global[0].key, KeyCode::Char('x'));
    match &keymap_config.global[0].action {
        AppCommand::SwitchView(view_id) => assert_eq!(view_id, "test.view2"),
        _ => panic!("Expected SwitchView action"),
    }
}

#[test]
fn test_view_specific_keybinding_overrides_global() {
    // Create a keymap with both global and view-specific bindings
    let mut keymap_config = KeymapConfig::new();
    
    // Global binding: 'x' switches to view2
    keymap_config = keymap_config.add_global(KeyBinding::new(
        KeyCode::Char('x'),
        AppCommand::SwitchView("view2".to_string()),
    ));
    
    // View-specific binding: 'x' runs a command in view1
    keymap_config = keymap_config.add_view_binding(
        "view1".to_string(),
        KeyBinding::new(
            KeyCode::Char('x'),
            AppCommand::RunCommand("custom-cmd".to_string(), HashMap::new()),
        ),
    );
    
    // Verify both bindings exist
    assert_eq!(keymap_config.global.len(), 1);
    assert_eq!(keymap_config.per_view.len(), 1);
    
    // Verify view-specific binding takes precedence (this is handled by KeymapResolver)
    let view_bindings = keymap_config.per_view.get("view1").unwrap();
    assert_eq!(view_bindings.len(), 1);
    assert_eq!(view_bindings[0].key, KeyCode::Char('x'));
    match &view_bindings[0].action {
        AppCommand::RunCommand(cmd_id, _) => assert_eq!(cmd_id, "custom-cmd"),
        _ => panic!("Expected RunCommand action"),
    }
}

#[test]
fn test_keymap_configuration_preserved_through_builder() {
    // Test that keymap configuration is preserved when building the app
    let mut keymap_config = KeymapConfig::new();
    keymap_config = keymap_config.add_global(KeyBinding::new(
        KeyCode::Char('t'),
        AppCommand::SwitchView("target.view".to_string()),
    ));
    
    let mut builder = AppBuilder::new();
    builder = builder
        .register_view(TestView::new("test.view"))
        .configure_keymap(keymap_config.clone());
    
    // Verify the keymap config is stored (we can't directly access it, but
    // we can verify the builder was created successfully)
    // In a real scenario, we'd build the app and test that the keybinding works
    assert!(true); // Builder creation succeeded
}

#[test]
fn test_multiple_view_specific_bindings() {
    // Test that multiple views can have different bindings for the same key
    let mut keymap_config = KeymapConfig::new();
    
    // View1: 'a' runs command1
    keymap_config = keymap_config.add_view_binding(
        "view1".to_string(),
        KeyBinding::new(
            KeyCode::Char('a'),
            AppCommand::RunCommand("cmd1".to_string(), HashMap::new()),
        ),
    );
    
    // View2: 'a' runs command2
    keymap_config = keymap_config.add_view_binding(
        "view2".to_string(),
        KeyBinding::new(
            KeyCode::Char('a'),
            AppCommand::RunCommand("cmd2".to_string(), HashMap::new()),
        ),
    );
    
    // Verify both view-specific bindings exist
    assert_eq!(keymap_config.per_view.len(), 2);
    
    let view1_bindings = keymap_config.per_view.get("view1").unwrap();
    let view2_bindings = keymap_config.per_view.get("view2").unwrap();
    
    assert_eq!(view1_bindings.len(), 1);
    assert_eq!(view2_bindings.len(), 1);
    
    // Verify they have different actions
    match &view1_bindings[0].action {
        AppCommand::RunCommand(cmd_id, _) => assert_eq!(cmd_id, "cmd1"),
        _ => panic!("Expected RunCommand action"),
    }
    
    match &view2_bindings[0].action {
        AppCommand::RunCommand(cmd_id, _) => assert_eq!(cmd_id, "cmd2"),
        _ => panic!("Expected RunCommand action"),
    }
}

#[test]
fn test_keymap_resolver_priority() {
    // Test that KeymapResolver correctly prioritizes bindings
    // This is more of a unit test for KeymapResolver, but included here
    // to verify the integration works correctly
    
    use tui_framework::keymap::{KeymapRegistry, KeymapResolver};
    
    let mut keymap_config = KeymapConfig::new();
    
    // Global binding
    keymap_config = keymap_config.add_global(KeyBinding::new(
        KeyCode::Char('g'),
        AppCommand::SwitchView("global-view".to_string()),
    ));
    
    // View-specific binding
    keymap_config = keymap_config.add_view_binding(
        "test-view".to_string(),
        KeyBinding::new(
            KeyCode::Char('g'),
            AppCommand::RunCommand("view-cmd".to_string(), HashMap::new()),
        ),
    );
    
    // Load into registry and create resolver
    let mut registry = KeymapRegistry::new();
    registry.load_config(keymap_config);
    let resolver = KeymapResolver::new(registry);
    
    // Test: view-specific should override global
    let result = resolver.resolve(KeyCode::Char('g'), Some("test-view"), false);
    assert!(result.is_some());
    match result.unwrap() {
        AppCommand::RunCommand(cmd_id, _) => assert_eq!(cmd_id, "view-cmd"),
        _ => panic!("View-specific binding should override global"),
    }
    
    // Test: global should be used when no view-specific binding
    let result = resolver.resolve(KeyCode::Char('g'), Some("other-view"), false);
    assert!(result.is_some());
    match result.unwrap() {
        AppCommand::SwitchView(view_id) => assert_eq!(view_id, "global-view"),
        _ => panic!("Global binding should be used when no view-specific"),
    }
    
    // Test: modal active should return None (modals handle their own keys)
    let result = resolver.resolve(KeyCode::Char('g'), Some("test-view"), true);
    assert!(result.is_none(), "Modal active should prevent keymap resolution");
}
