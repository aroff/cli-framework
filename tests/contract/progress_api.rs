//! Contract tests for progress reporting API

use tui_framework::app::background_tasks::{BackgroundTaskManager, ProgressReporter};

#[test]
fn test_progress_reporter_new() {
    // Contract: new() creates ProgressReporter with current, total, and None message
    let progress = ProgressReporter::new(45, 200);
    assert_eq!(progress.current, 45);
    assert_eq!(progress.total, Some(200));
    assert_eq!(progress.message, None);
}

#[test]
fn test_progress_reporter_with_message() {
    // Contract: with_message() creates ProgressReporter with message set
    let progress = ProgressReporter::with_message(45, 200, "Processing file.jpg");
    assert_eq!(progress.current, 45);
    assert_eq!(progress.total, Some(200));
    assert_eq!(progress.message, Some("Processing file.jpg".to_string()));
}

#[test]
fn test_progress_reporter_percentage() {
    // Contract: percentage() calculates completion percentage
    let progress = ProgressReporter::new(45, 200);
    assert_eq!(progress.percentage(), 22.5);

    // Contract: percentage() caps at 100% when current > total
    let progress = ProgressReporter::new(200, 150);
    assert_eq!(progress.percentage(), 100.0);

    // Contract: percentage() returns 0.0 if total is None or 0
    let mut progress = ProgressReporter::new(45, 0);
    progress.total = None;
    assert_eq!(progress.percentage(), 0.0);
}

#[test]
fn test_progress_reporter_is_complete() {
    // Contract: is_complete() returns true when current >= total
    let progress = ProgressReporter::new(200, 200);
    assert!(progress.is_complete());

    // Contract: is_complete() returns false when current < total
    let progress = ProgressReporter::new(45, 200);
    assert!(!progress.is_complete());

    // Contract: is_complete() returns false when total is None
    let mut progress = ProgressReporter::new(45, 0);
    progress.total = None;
    assert!(!progress.is_complete());
}

#[tokio::test]
async fn test_spawn_with_progress() {
    // Contract: spawn_with_progress() returns CancellationToken and Receiver
    let mut manager = BackgroundTaskManager::new();
    let (token, mut progress_rx) = manager.spawn_with_progress(|progress_tx, _cancel_token| {
        Box::pin(async move {
            let progress = ProgressReporter::new(1, 10);
            let _ = progress_tx.send(progress).await;
            Ok(())
        })
    });

    // Verify we got a token and receiver
    assert!(!token.is_cancelled());

    // Wait a bit for the task to send progress
    tokio::time::sleep(tokio::time::Duration::from_millis(10)).await;

    // Verify we can receive progress updates
    let progress = progress_rx.try_recv();
    assert!(progress.is_ok());
    if let Ok(p) = progress {
        assert_eq!(p.current, 1);
        assert_eq!(p.total, Some(10));
    }
}

#[tokio::test]
async fn test_backward_compatibility() {
    // Contract: Verify backward compatibility (existing BackgroundTaskManager methods unchanged)
    let mut manager = BackgroundTaskManager::new();

    // Verify existing methods still exist and work
    let token = manager.spawn(async { Ok(()) });
    assert!(!token.is_cancelled());

    let token2 = manager.spawn_streaming(|_stream_tx, _cancel_token| Box::pin(async { Ok(()) }));
    assert!(!token2.is_cancelled());

    // Verify new method doesn't break existing functionality
    let (token3, _progress_rx) =
        manager.spawn_with_progress(|_progress_tx, _cancel_token| Box::pin(async { Ok(()) }));
    assert!(!token3.is_cancelled());

    // All methods should coexist
    assert!(manager.active_task_count() >= 3);
}
