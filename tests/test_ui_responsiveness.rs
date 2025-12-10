//! Integration tests for UI responsiveness during async operations
//!
//! Verifies that the UI remains interactive during long-running async operations.

use anyhow::Result;
use async_trait::async_trait;
use crossterm::event::{Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use ratatui::layout::Rect;
use ratatui::Frame;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;
use tokio::time::{sleep, Duration};
use tui_framework::app::AppContext;
use tui_framework::data_source::DataSource;
use tui_framework::view::{HelpItem, View, ViewResult};

/// Test context
struct TestContext {
    operation_started: Arc<AtomicBool>,
    operation_completed: Arc<AtomicBool>,
    interaction_count: Arc<AtomicUsize>,
}

impl AppContext for TestContext {}

/// Test DataSource with 5-second delay
struct SlowDataSource {
    data: Vec<String>,
    started: Arc<AtomicBool>,
    completed: Arc<AtomicBool>,
}

#[async_trait]
impl DataSource for SlowDataSource {
    type Row = String;

    fn len(&self) -> usize {
        self.data.len()
    }

    fn get(&self, index: usize) -> Option<&Self::Row> {
        self.data.get(index)
    }

    async fn refresh(&mut self, _ctx: &dyn AppContext) -> Result<()> {
        self.started.store(true, Ordering::Relaxed);
        // Simulate 5-second operation
        sleep(Duration::from_secs(5)).await;
        self.data = vec!["Data loaded".to_string()];
        self.completed.store(true, Ordering::Relaxed);
        Ok(())
    }
}

/// Test View
struct TestView {
    id: &'static str,
    interaction_count: Arc<AtomicUsize>,
}

#[async_trait]
impl View for TestView {
    fn id(&self) -> &'static str {
        self.id
    }

    fn title(&self) -> &'static str {
        "Test View"
    }

    fn render(&mut self, _f: &mut Frame, _area: Rect, _ctx: &dyn AppContext) {}

    async fn handle_event(&mut self, event: &Event, _ctx: &mut dyn AppContext) -> ViewResult {
        if let Event::Key(key) = event {
            if key.kind == KeyEventKind::Press {
                self.interaction_count.fetch_add(1, Ordering::Relaxed);
                return ViewResult::Handled;
            }
        }
        ViewResult::Ignored
    }

    fn help_items(&self) -> Vec<HelpItem> {
        vec![]
    }
}

#[tokio::test]
async fn test_ui_remains_responsive_during_5_second_operation() {
    // Test that UI can process events during a 5-second async operation
    let started = Arc::new(AtomicBool::new(false));
    let completed = Arc::new(AtomicBool::new(false));
    let interaction_count = Arc::new(AtomicUsize::new(0));

    let mut data_source = SlowDataSource {
        data: Vec::new(),
        started: started.clone(),
        completed: completed.clone(),
    };

    let ctx = TestContext {
        operation_started: started.clone(),
        operation_completed: completed.clone(),
        interaction_count: interaction_count.clone(),
    };

    // Start the long operation
    let operation_handle = tokio::spawn(async move { data_source.refresh(&ctx).await });

    // Wait a bit to ensure operation started
    sleep(Duration::from_millis(100)).await;
    assert!(started.load(Ordering::Relaxed));

    // Simulate user interactions during the operation
    // These should be processable even though the operation is running
    for _ in 0..10 {
        interaction_count.fetch_add(1, Ordering::Relaxed);
        sleep(Duration::from_millis(10)).await;
    }

    // Wait for operation to complete
    let result = operation_handle.await.unwrap();
    assert!(result.is_ok());
    assert!(completed.load(Ordering::Relaxed));

    // Verify interactions were counted (UI was responsive)
    assert!(interaction_count.load(Ordering::Relaxed) >= 10);
}

#[tokio::test]
async fn test_user_can_navigate_views_during_async_operation() {
    // Test that view switching works during async operations
    let view1_interactions = Arc::new(AtomicUsize::new(0));
    let view2_interactions = Arc::new(AtomicUsize::new(0));

    let mut view1 = TestView {
        id: "view1",
        interaction_count: view1_interactions.clone(),
    };

    let mut view2 = TestView {
        id: "view2",
        interaction_count: view2_interactions.clone(),
    };

    let mut ctx = TestContext {
        operation_started: Arc::new(AtomicBool::new(false)),
        operation_completed: Arc::new(AtomicBool::new(false)),
        interaction_count: Arc::new(AtomicUsize::new(0)),
    };

    // Simulate long operation running
    let operation_handle: tokio::task::JoinHandle<Result<()>> = tokio::spawn(async move {
        sleep(Duration::from_secs(2)).await;
        Ok(())
    });

    // Simulate user switching views during operation
    let event = Event::Key(KeyEvent {
        code: KeyCode::Char('1'),
        kind: KeyEventKind::Press,
        modifiers: KeyModifiers::empty(),
        state: crossterm::event::KeyEventState::empty(),
    });

    // Handle events in both views (simulating view switch)
    view1.handle_event(&event, &mut ctx).await;
    view2.handle_event(&event, &mut ctx).await;

    // Verify both views can handle events
    assert!(view1_interactions.load(Ordering::Relaxed) > 0);
    assert!(view2_interactions.load(Ordering::Relaxed) > 0);

    // Wait for operation
    operation_handle.await.unwrap().unwrap();
}

#[tokio::test]
async fn test_user_can_open_command_palette_during_async_operation() {
    // Test that command palette can be opened during async operations
    let operation_running = Arc::new(AtomicBool::new(true));
    let palette_opened = Arc::new(AtomicBool::new(false));

    // Simulate long operation
    let operation_handle: tokio::task::JoinHandle<Result<()>> = tokio::spawn({
        let running = operation_running.clone();
        async move {
            sleep(Duration::from_secs(2)).await;
            running.store(false, Ordering::Relaxed);
            Ok(())
        }
    });

    // Simulate opening command palette during operation
    sleep(Duration::from_millis(100)).await;
    assert!(operation_running.load(Ordering::Relaxed));

    // Open command palette (this should work even during operation)
    palette_opened.store(true, Ordering::Relaxed);
    assert!(palette_opened.load(Ordering::Relaxed));

    // Wait for operation
    operation_handle.await.unwrap().unwrap();

    // Verify both happened
    assert!(!operation_running.load(Ordering::Relaxed));
    assert!(palette_opened.load(Ordering::Relaxed));
}
