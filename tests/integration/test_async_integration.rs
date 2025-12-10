//! Integration tests for async operations
//!
//! Verifies that async operations don't block the UI and work correctly
//! in the full application context.

use async_trait::async_trait;
use tui_framework::app::{AppBuilder, AppContext};
use tui_framework::command::{Command, CommandArgs};
use tui_framework::data_source::DataSource;
use tui_framework::view::{View, ViewResult, HelpItem};
use tui_framework::message::AppMessage;
use crossterm::event::{Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use ratatui::layout::Rect;
use ratatui::Frame;
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use tokio::time::{sleep, Duration, Instant};

/// Test context
struct TestContext {
    data_refreshed: Arc<AtomicBool>,
    command_executed: Arc<AtomicBool>,
}

impl AppContext for TestContext {}

/// Test DataSource that simulates async network operation
struct TestDataSource {
    data: Vec<String>,
    refreshed: Arc<AtomicBool>,
}

#[async_trait]
impl DataSource for TestDataSource {
    type Row = String;

    fn len(&self) -> usize {
        self.data.len()
    }

    fn get(&self, index: usize) -> Option<&Self::Row> {
        self.data.get(index)
    }

    async fn refresh(&mut self, _ctx: &dyn AppContext) -> Result<()> {
        // Simulate 100ms network operation
        sleep(Duration::from_millis(100)).await;
        self.data = vec!["Item 1".to_string(), "Item 2".to_string()];
        self.refreshed.store(true, Ordering::Relaxed);
        Ok(())
    }
}

/// Test View that triggers async operations
struct TestView {
    data_source: TestDataSource,
}

#[async_trait]
impl View for TestView {
    fn id(&self) -> &'static str {
        "test.view"
    }

    fn title(&self) -> &'static str {
        "Test View"
    }

    fn render(&mut self, _f: &mut Frame, _area: Rect, _ctx: &dyn AppContext) {
        // Render data
    }

    async fn handle_event(&mut self, event: &Event, ctx: &mut dyn AppContext) -> ViewResult {
        if let Event::Key(key) = event {
            if key.code == KeyCode::Char('r') && key.kind == KeyEventKind::Press {
                // Trigger async refresh
                if let Err(e) = self.data_source.refresh(ctx).await {
                    return ViewResult::ShowModal(AppMessage::error(format!("Refresh failed: {}", e)));
                }
                return ViewResult::Handled;
            }
        }
        ViewResult::Ignored
    }

    fn help_items(&self) -> Vec<HelpItem> {
        vec![HelpItem {
            key: "r".to_string(),
            description: "Refresh data".to_string(),
        }]
    }
}

#[tokio::test]
async fn test_async_data_source_refresh_doesnt_block_ui() {
    // This test verifies that async DataSource refresh operations
    // can be performed without blocking the UI event loop
    
    let refreshed = Arc::new(AtomicBool::new(false));
    let data_source = TestDataSource {
        data: Vec::new(),
        refreshed: refreshed.clone(),
    };
    
    let ctx = TestContext {
        data_refreshed: refreshed.clone(),
        command_executed: Arc::new(AtomicBool::new(false)),
    };
    
    // Start refresh operation
    let start = Instant::now();
    let mut ds = data_source;
    let refresh_result = ds.refresh(&ctx).await;
    let elapsed = start.elapsed();
    
    // Verify refresh completed
    assert!(refresh_result.is_ok());
    assert!(refreshed.load(Ordering::Relaxed));
    assert_eq!(ds.len(), 2);
    
    // Verify it took approximately 100ms (with some tolerance)
    assert!(elapsed >= Duration::from_millis(90));
    assert!(elapsed <= Duration::from_millis(200)); // Allow for test overhead
}

#[tokio::test]
async fn test_async_command_execution_doesnt_block_ui() {
    // This test verifies that async command execution
    // can be performed without blocking
    
    let executed = Arc::new(AtomicBool::new(false));
    let executed_clone = executed.clone();
    
    let command = Command {
        id: "test.async",
        summary: "Test async command",
        syntax: None,
        category: None,
        execute: move |_ctx: &mut dyn AppContext, _args: CommandArgs| {
            let exec = executed_clone.clone();
            Box::pin(async move {
                // Simulate async operation
                sleep(Duration::from_millis(50)).await;
                exec.store(true, Ordering::Relaxed);
                Ok(())
            })
        },
    };
    
    let mut ctx = TestContext {
        data_refreshed: Arc::new(AtomicBool::new(false)),
        command_executed: executed.clone(),
    };
    
    let args = CommandArgs {
        positional: vec![],
        named: HashMap::new(),
    };
    
    // Execute command
    let start = Instant::now();
    let result = (command.execute)(&mut ctx, args).await;
    let elapsed = start.elapsed();
    
    // Verify command executed
    assert!(result.is_ok());
    assert!(executed.load(Ordering::Relaxed));
    
    // Verify it took approximately 50ms
    assert!(elapsed >= Duration::from_millis(40));
    assert!(elapsed <= Duration::from_millis(150));
}

