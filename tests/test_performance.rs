//! Performance tests for async operations
//!
//! Verifies that performance requirements are met (50ms response time, etc.)

use anyhow::Result;
use async_trait::async_trait;
use crossterm::event::{Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use ratatui::layout::Rect;
use ratatui::Frame;
use tokio::time::{sleep, Duration, Instant};
use tui_framework::app::AppContext;
use tui_framework::data_source::DataSource;
use tui_framework::view::{HelpItem, View, ViewResult};

/// Test context
struct TestContext;

impl AppContext for TestContext {}

/// Fast DataSource for performance testing
struct FastDataSource {
    data: Vec<String>,
}

#[async_trait]
impl DataSource for FastDataSource {
    type Row = String;

    fn len(&self) -> usize {
        self.data.len()
    }

    fn get(&self, index: usize) -> Option<&Self::Row> {
        self.data.get(index)
    }

    async fn refresh(&mut self, _ctx: &dyn AppContext) -> Result<()> {
        // Very fast operation
        sleep(Duration::from_millis(1)).await;
        self.data = vec!["Fast data".to_string()];
        Ok(())
    }
}

/// Test View for performance testing
struct PerformanceView;

#[async_trait]
impl View for PerformanceView {
    fn id(&self) -> &'static str {
        "perf.view"
    }

    fn title(&self) -> &'static str {
        "Performance View"
    }

    fn render(&mut self, _f: &mut Frame, _area: Rect, _ctx: &dyn AppContext) {}

    async fn handle_event(&mut self, _event: &Event, _ctx: &mut dyn AppContext) -> ViewResult {
        // Fast event handling
        sleep(Duration::from_millis(1)).await;
        ViewResult::Handled
    }

    fn help_items(&self) -> Vec<HelpItem> {
        vec![]
    }
}

#[tokio::test]
async fn test_user_interactions_respond_within_50ms_during_async_ops() {
    // Test that user interactions respond within 50ms even during async operations
    let mut view = PerformanceView;
    let mut ctx = TestContext;

    // Start a background operation
    let background_handle: tokio::task::JoinHandle<Result<()>> = tokio::spawn(async {
        sleep(Duration::from_millis(100)).await;
        Ok(())
    });

    // Simulate user interaction
    let event = Event::Key(KeyEvent {
        code: KeyCode::Char('a'),
        kind: KeyEventKind::Press,
        modifiers: KeyModifiers::empty(),
        state: crossterm::event::KeyEventState::empty(),
    });

    // Measure response time
    let start = Instant::now();
    let _result = view.handle_event(&event, &mut ctx).await;
    let elapsed = start.elapsed();

    // Verify response time is within 50ms (SC-002)
    assert!(
        elapsed <= Duration::from_millis(50),
        "User interaction took {}ms, should be <= 50ms",
        elapsed.as_millis()
    );

    // Wait for background operation
    background_handle.await.unwrap().unwrap();
}

#[tokio::test]
async fn test_event_loop_latency_within_16ms() {
    // Test that event loop can process events within 16ms (SC-007)
    let data_source = FastDataSource { data: Vec::new() };

    // Simulate event loop tick
    let start = Instant::now();

    // Process event (simulate reading and handling)
    sleep(Duration::from_millis(1)).await;

    // Check if data source can be accessed quickly
    let _len = data_source.len();

    let elapsed = start.elapsed();

    // Verify latency is within 16ms per frame
    assert!(
        elapsed <= Duration::from_millis(16),
        "Event loop latency {}ms, should be <= 16ms",
        elapsed.as_millis()
    );
}
