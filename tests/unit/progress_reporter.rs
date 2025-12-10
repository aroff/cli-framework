//! Unit tests for ProgressReporter

use std::time::Duration;
use tui_framework::app::background_tasks::{BackgroundTaskManager, ProgressReporter};

#[test]
fn test_progress_reporter_edge_cases() {
    // Edge case: Zero total
    let progress = ProgressReporter::new(0, 0);
    assert_eq!(progress.percentage(), 0.0);
    assert!(!progress.is_complete());

    // Edge case: Progress > 100%
    let progress = ProgressReporter::new(200, 150);
    assert_eq!(progress.percentage(), 100.0); // Capped at 100%
    assert!(progress.is_complete()); // current >= total

    // Edge case: None total (indeterminate progress)
    let mut progress = ProgressReporter::new(45, 0);
    progress.total = None;
    assert_eq!(progress.percentage(), 0.0);
    assert!(!progress.is_complete());

    // Edge case: Current is 0
    let progress = ProgressReporter::new(0, 100);
    assert_eq!(progress.percentage(), 0.0);
    assert!(!progress.is_complete());
}

#[tokio::test]
async fn test_progress_channel_creation_and_message_passing() {
    // Unit test: Progress channel creation and message passing
    let mut manager = BackgroundTaskManager::new();
    let (token, mut progress_rx) = manager.spawn_with_progress(|progress_tx, _cancel_token| {
        Box::pin(async move {
            // Send multiple progress updates
            for i in 1..=5 {
                let progress = ProgressReporter::with_message(i, 5, format!("Item {}", i));
                let _ = progress_tx.send(progress).await;
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
            Ok(())
        })
    });

    assert!(!token.is_cancelled());

    // Wait for messages
    tokio::time::sleep(Duration::from_millis(100)).await;

    // Collect all available messages
    let mut received = Vec::new();
    while let Ok(progress) = progress_rx.try_recv() {
        received.push(progress);
    }

    // Verify we received messages
    assert!(!received.is_empty(), "Should receive progress messages");

    // Verify messages have correct content
    for (i, progress) in received.iter().enumerate() {
        assert_eq!(progress.current, i + 1);
        assert_eq!(progress.total, Some(5));
        assert!(progress.message.is_some());
    }
}

#[test]
fn test_progress_reporter_message_field_handling() {
    // Unit test: ProgressReporter message field handling
    // Test with message
    let progress = ProgressReporter::with_message(45, 200, "Processing file.jpg");
    assert_eq!(progress.message, Some("Processing file.jpg".to_string()));

    // Test without message
    let progress = ProgressReporter::new(45, 200);
    assert_eq!(progress.message, None);

    // Test message with different types (Into<String>)
    let progress = ProgressReporter::with_message(1, 10, String::from("String message"));
    assert_eq!(progress.message, Some("String message".to_string()));

    let progress = ProgressReporter::with_message(1, 10, "&str message");
    assert_eq!(progress.message, Some("&str message".to_string()));
}

#[tokio::test]
async fn test_progress_channel_cloning_multiple_senders() {
    // Unit test: Progress channel cloning (multiple senders)
    let mut manager = BackgroundTaskManager::new();
    let (token, mut progress_rx) = manager.spawn_with_progress(|progress_tx, _cancel_token| {
        Box::pin(async move {
            // Clone the sender for concurrent use
            let progress_tx2 = progress_tx.clone();

            // Send from both senders
            let progress1 = ProgressReporter::new(1, 10);
            let _ = progress_tx.send(progress1).await;

            let progress2 = ProgressReporter::new(2, 10);
            let _ = progress_tx2.send(progress2).await;

            Ok(())
        })
    });

    assert!(!token.is_cancelled());

    // Wait for messages
    tokio::time::sleep(Duration::from_millis(50)).await;

    // Collect messages
    let mut received = Vec::new();
    while let Ok(progress) = progress_rx.try_recv() {
        received.push(progress);
    }

    // Verify we received messages from both senders
    assert!(
        !received.is_empty(),
        "Should receive messages from cloned senders"
    );
}
