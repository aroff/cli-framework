//! Contract tests for Module trait implementations
//!
//! These tests verify that Module implementations comply with the Module trait contract
//! as defined in contracts/module-trait.md

use tui_framework::prelude::*;
use tui_framework::app::Module;
use tui_framework::view::{View, ViewResult, HelpItem};
use tui_framework::command::Command;
use crossterm::event::Event;
use ratatui::layout::Rect;
use ratatui::Frame;
use anyhow::Result;

// Test view for module tests
struct TestModuleView;

impl View for TestModuleView {
    fn id(&self) -> &'static str {
        "module.test.view"
    }

    fn title(&self) -> &'static str {
        "Test View"
    }

    fn render(&mut self, _f: &mut Frame, _area: Rect, _ctx: &dyn AppContext) {}
    fn handle_event(&mut self, _event: &Event, _ctx: &mut dyn AppContext) -> ViewResult {
        ViewResult::Ignored
    }
    fn help_items(&self) -> Vec<HelpItem> {
        vec![]
    }
}

// Test module implementation
struct TestModule;

impl Module for TestModule {
    fn id(&self) -> &'static str {
        "test-module"
    }

    fn register(&self, builder: &mut AppBuilder) -> Result<()> {
        builder.register_view(TestModuleView);
        Ok(())
    }
}

#[test]
fn test_module_id_returns_non_empty_string() {
    let module = TestModule;
    let id = module.id();
    
    assert!(!id.is_empty(), "Module ID must not be empty");
    assert_eq!(id, "test-module");
}

#[test]
fn test_module_register_succeeds() {
    let module = TestModule;
    let mut builder = AppBuilder::new();
    
    let result = module.register(&mut builder);
    assert!(result.is_ok(), "Module registration should succeed");
}

#[test]
fn test_module_register_adds_components() {
    let module = TestModule;
    let mut builder = AppBuilder::new();
    
    module.register(&mut builder).unwrap();
    
    // Verify view was registered by trying to build (would fail if view not registered)
    struct TestContext;
    impl AppContext for TestContext {}
    
    let result = builder.build(TestContext);
    assert!(result.is_ok(), "Builder should succeed after module registration");
}

#[test]
fn test_module_can_register_multiple_components() {
    struct MultiComponentModule;
    
    impl Module for MultiComponentModule {
        fn id(&self) -> &'static str {
            "multi-component"
        }
        
        fn register(&self, builder: &mut AppBuilder) -> Result<()> {
            builder
                .register_view(TestModuleView)
                .register_command(Command {
                    id: "test-cmd",
                    summary: "Test command",
                    syntax: None,
                    category: None,
                    execute: |_ctx, _args| Ok(()),
                });
            Ok(())
        }
    }
    
    let module = MultiComponentModule;
    let mut builder = AppBuilder::new();
    
    let result = module.register(&mut builder);
    assert!(result.is_ok(), "Module should register multiple components");
}

#[test]
fn test_module_id_is_stable() {
    let module1 = TestModule;
    let module2 = TestModule;
    
    assert_eq!(module1.id(), module2.id(), "Module ID should be stable");
    assert_eq!(module1.id(), "test-module");
}
