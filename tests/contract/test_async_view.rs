//! Contract tests for async View trait
//!
//! Verifies that View implementations correctly implement the async handle_event contract.

use async_trait::async_trait;
use crossterm::event::{Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use ratatui::layout::Rect;
use ratatui::Frame;
use tokio::time::{sleep, Duration};
use tui_framework::app::AppContext;
use tui_framework::view::{HelpItem, View, ViewResult};

/// Test context for async View tests
struct TestContext {
    #[allow(dead_code)]
    event_handled: bool,
}

impl AppContext for TestContext {}

/// Test View that performs async operations in handle_event
struct AsyncTestView {
    data: String,
}

#[async_trait]
impl View for AsyncTestView {
    fn id(&self) -> &'static str {
        "async.test.view"
    }

    fn title(&self) -> &'static str {
        "Async Test View"
    }

    fn render(&mut self, _f: &mut Frame, _area: Rect, _ctx: &dyn AppContext) {
        // Synchronous rendering
    }

    async fn handle_event(&mut self, event: &Event, _ctx: &mut dyn AppContext) -> ViewResult {
        if let Event::Key(key) = event {
            if key.code == KeyCode::Char('r') && key.kind == KeyEventKind::Press {
                // Simulate async operation (e.g., refresh data)
                sleep(Duration::from_millis(10)).await;
                self.data = "Refreshed".to_string();
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
async fn test_async_view_handle_event_is_async() {
    let mut view = AsyncTestView {
        data: "Initial".to_string(),
    };

    let mut ctx = TestContext {
        event_handled: false,
    };

    // Create a key event
    let event = Event::Key(KeyEvent {
        code: KeyCode::Char('r'),
        kind: KeyEventKind::Press,
        modifiers: KeyModifiers::empty(),
        state: crossterm::event::KeyEventState::empty(),
    });

    // Handle event asynchronously
    let start = std::time::Instant::now();
    let result = view.handle_event(&event, &mut ctx).await;
    let elapsed = start.elapsed();

    // Verify it completed asynchronously
    assert!(elapsed >= Duration::from_millis(10));

    // Verify event was handled
    assert!(matches!(result, ViewResult::Handled));

    // Verify data was updated
    assert_eq!(view.data, "Refreshed");
}

#[tokio::test]
async fn test_async_view_handle_event_can_switch_views() {
    struct SwitchView;

    #[async_trait]
    impl View for SwitchView {
        fn id(&self) -> &'static str {
            "switch.view"
        }

        fn title(&self) -> &'static str {
            "Switch View"
        }

        fn render(&mut self, _f: &mut Frame, _area: Rect, _ctx: &dyn AppContext) {}

        async fn handle_event(&mut self, event: &Event, _ctx: &mut dyn AppContext) -> ViewResult {
            if let Event::Key(key) = event {
                if key.code == KeyCode::Char('s') && key.kind == KeyEventKind::Press {
                    // Perform async operation then switch view
                    sleep(Duration::from_millis(5)).await;
                    return ViewResult::SwitchView("other.view".to_string());
                }
            }
            ViewResult::Ignored
        }

        fn help_items(&self) -> Vec<HelpItem> {
            vec![]
        }
    }

    let mut view = SwitchView;
    let mut ctx = TestContext {
        event_handled: false,
    };

    let event = Event::Key(KeyEvent {
        code: KeyCode::Char('s'),
        kind: KeyEventKind::Press,
        modifiers: KeyModifiers::empty(),
        state: crossterm::event::KeyEventState::empty(),
    });

    let result = view.handle_event(&event, &mut ctx).await;

    if let ViewResult::SwitchView(view_id) = result {
        assert_eq!(view_id, "other.view");
    } else {
        panic!("Expected SwitchView result");
    }
}

#[tokio::test]
async fn test_async_view_handle_event_can_show_modal() {
    struct ModalView;

    #[async_trait]
    impl View for ModalView {
        fn id(&self) -> &'static str {
            "modal.view"
        }

        fn title(&self) -> &'static str {
            "Modal View"
        }

        fn render(&mut self, _f: &mut Frame, _area: Rect, _ctx: &dyn AppContext) {}

        async fn handle_event(&mut self, event: &Event, _ctx: &mut dyn AppContext) -> ViewResult {
            if let Event::Key(key) = event {
                if key.code == KeyCode::Char('m') && key.kind == KeyEventKind::Press {
                    // Perform async operation then show modal
                    sleep(Duration::from_millis(5)).await;
                    return ViewResult::ShowModal(AppMessage::info("Async operation complete"));
                }
            }
            ViewResult::Ignored
        }

        fn help_items(&self) -> Vec<HelpItem> {
            vec![]
        }
    }

    let mut view = ModalView;
    let mut ctx = TestContext {
        event_handled: false,
    };

    let event = Event::Key(KeyEvent {
        code: KeyCode::Char('m'),
        kind: KeyEventKind::Press,
        modifiers: KeyModifiers::empty(),
        state: crossterm::event::KeyEventState::empty(),
    });

    let result = view.handle_event(&event, &mut ctx).await;

    if let ViewResult::ShowModal(msg) = result {
        assert_eq!(msg.short, "Async operation complete");
    } else {
        panic!("Expected ShowModal result");
    }
}
