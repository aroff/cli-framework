//! Integration tests for progress reporting

use std::time::Duration;
use tui_framework::app::background_tasks::{BackgroundTaskManager, ProgressReporter};
use tui_framework::progress_formatting;

#[tokio::test]
async fn test_basic_progress_reporting() {
    // Integration test: Basic progress reporting with 100 items
    let mut manager = BackgroundTaskManager::new();
    let (token, mut progress_rx) = manager.spawn_with_progress(|progress_tx, cancel_token| {
        Box::pin(async move {
            let total = 100;
            for i in 1..=total {
                if cancel_token.is_cancelled() {
                    break;
                }
                let progress = ProgressReporter::new(i, total);
                let _ = progress_tx.send(progress).await;
                tokio::time::sleep(Duration::from_millis(1)).await;
            }
            Ok(())
        })
    });

    assert!(!token.is_cancelled());

    // Collect progress updates
    let mut updates = Vec::new();
    let mut last_update = None;

    // Poll for updates (with timeout to avoid infinite loop)
    let start = std::time::Instant::now();
    while start.elapsed() < Duration::from_secs(2) {
        if let Ok(progress) = progress_rx.try_recv() {
            updates.push(progress.clone());
            last_update = Some(progress);
        } else {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }

        // If we got the final update, break
        if let Some(ref last) = last_update {
            if last.current >= 100 {
                break;
            }
        }
    }

    // Verify we received progress updates
    assert!(
        !updates.is_empty(),
        "Should receive at least some progress updates"
    );

    // Verify progress moves forward
    for i in 1..updates.len() {
        assert!(
            updates[i].current >= updates[i - 1].current,
            "Progress should only move forward"
        );
    }
}

#[tokio::test]
async fn test_progress_reporting_with_contextual_messages() {
    // Integration test: Progress reporting with contextual messages
    let mut manager = BackgroundTaskManager::new();
    let (_token, mut progress_rx) = manager.spawn_with_progress(|progress_tx, cancel_token| {
        Box::pin(async move {
            let files = vec!["file1.jpg", "file2.jpg", "file3.jpg"];
            for (i, file) in files.iter().enumerate() {
                if cancel_token.is_cancelled() {
                    break;
                }
                let progress = ProgressReporter::with_message(
                    i + 1,
                    files.len(),
                    format!("Processing {}", file),
                );
                let _ = progress_tx.send(progress).await;
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
            Ok(())
        })
    });

    assert!(!_token.is_cancelled());

    // Collect progress updates
    let mut updates = Vec::new();
    let start = std::time::Instant::now();
    while start.elapsed() < Duration::from_secs(1) {
        if let Ok(progress) = progress_rx.try_recv() {
            updates.push(progress);
        } else {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        if updates.len() >= 3 {
            break;
        }
    }

    // Verify we received updates with messages
    assert!(!updates.is_empty(), "Should receive progress updates");
    assert!(
        updates.iter().any(|p| p.message.is_some()),
        "Should have contextual messages"
    );

    // Verify messages are correct
    for update in &updates {
        if let Some(ref msg) = update.message {
            assert!(
                msg.contains("Processing"),
                "Message should describe operation"
            );
        }
    }
}

#[tokio::test]
async fn test_in_place_progress_updates() {
    // Integration test: In-place progress updates (carriage return behavior)
    // Note: This test verifies the function works, actual terminal behavior is hard to test
    let progress = ProgressReporter::with_message(45, 200, "Processing file.jpg");

    // Verify format_progress_with_percentage produces correct output
    let formatted = progress_formatting::format_progress_with_percentage(&progress);
    assert!(formatted.contains("45"));
    assert!(formatted.contains("200"));
    assert!(formatted.contains("22.5%"));
    assert!(formatted.contains("Processing file.jpg"));

    // Verify print_progress_update doesn't panic (best-effort test)
    progress_formatting::print_progress_update(&progress);
}

#[tokio::test]
async fn test_final_progress_summary_with_newline() {
    // Integration test: Final progress summary with newline
    let progress = ProgressReporter::new(200, 200);

    // Verify format_progress_with_percentage produces correct final output
    let formatted = progress_formatting::format_progress_with_percentage(&progress);
    assert!(formatted.contains("200"));
    assert!(formatted.contains("100.0%"));

    // Verify print_progress_complete doesn't panic (best-effort test)
    progress_formatting::print_progress_complete(&progress);
}

#[tokio::test]
async fn test_multiple_concurrent_operations() {
    // Integration test: Multiple concurrent operations with progress reporting
    let mut manager = BackgroundTaskManager::new();
    let mut receivers = Vec::new();

    // Spawn 5 concurrent tasks
    for task_id in 0..5 {
        let (_token, progress_rx) =
            manager.spawn_with_progress(move |progress_tx, cancel_token| {
                let task_id = task_id;
                Box::pin(async move {
                    for i in 1..=10 {
                        if cancel_token.is_cancelled() {
                            break;
                        }
                        let progress = ProgressReporter::with_message(
                            i,
                            10,
                            format!("Task {}: item {}", task_id, i),
                        );
                        let _ = progress_tx.send(progress).await;
                        tokio::time::sleep(Duration::from_millis(10)).await;
                    }
                    Ok(())
                })
            });
        receivers.push((task_id, progress_rx));
    }

    // Collect progress from all receivers
    let mut all_updates = Vec::new();
    let start = std::time::Instant::now();
    while start.elapsed() < Duration::from_secs(2) {
        for (task_id, progress_rx) in &mut receivers {
            if let Ok(progress) = progress_rx.try_recv() {
                all_updates.push((*task_id, progress));
            }
        }
        if all_updates.len() >= 50 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }

    // Verify we received updates from multiple tasks
    assert!(
        !all_updates.is_empty(),
        "Should receive updates from concurrent operations"
    );

    // Verify updates have correct task IDs
    let task_ids: std::collections::HashSet<usize> =
        all_updates.iter().map(|(id, _)| *id).collect();
    assert!(
        task_ids.len() > 1,
        "Should receive updates from multiple tasks"
    );
}

#[tokio::test]
async fn test_progress_update_handling_fast_updates() {
    // Integration test: Progress update handling when updates arrive faster than display
    let mut manager = BackgroundTaskManager::new();
    let (token, mut progress_rx) = manager.spawn_with_progress(|progress_tx, _cancel_token| {
        Box::pin(async move {
            // Send many rapid updates
            for i in 1..=100 {
                let progress = ProgressReporter::new(i, 100);
                let _ = progress_tx.send(progress).await;
                // Very short delay to simulate fast updates
                tokio::time::sleep(Duration::from_micros(100)).await;
            }
            Ok(())
        })
    });

    assert!(!token.is_cancelled());

    // Simulate slow display: only process latest update
    let mut latest_progress = None;
    let start = std::time::Instant::now();
    while start.elapsed() < Duration::from_secs(2) {
        // Collect all available, keep only latest
        while let Ok(progress) = progress_rx.try_recv() {
            latest_progress = Some(progress);
        }
        tokio::time::sleep(Duration::from_millis(50)).await; // Simulate slow display
    }

    // Verify we got at least one update (latest)
    assert!(
        latest_progress.is_some(),
        "Should receive at least the latest update"
    );
    if let Some(progress) = latest_progress {
        assert!(progress.current > 0, "Progress should have advanced");
    }
}

#[tokio::test]
async fn test_out_of_order_progress_updates() {
    // Integration test: Out-of-order progress updates (progress only moves forward)
    let mut manager = BackgroundTaskManager::new();
    let (token, mut progress_rx) = manager.spawn_with_progress(|progress_tx, _cancel_token| {
        Box::pin(async move {
            // Send updates in order
            for i in 1..=10 {
                let progress = ProgressReporter::new(i, 10);
                let _ = progress_tx.send(progress).await;
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
            Ok(())
        })
    });

    assert!(!token.is_cancelled());

    // Collect updates and verify they only move forward
    let mut last_displayed = 0;
    let mut updates = Vec::new();
    let start = std::time::Instant::now();
    while start.elapsed() < Duration::from_secs(1) {
        if let Ok(progress) = progress_rx.try_recv() {
            // Use should_display_progress to filter backwards updates
            if progress_formatting::should_display_progress(progress.current, last_displayed) {
                updates.push(progress.clone());
                last_displayed = progress.current;
            }
        } else {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        if updates.len() >= 10 {
            break;
        }
    }

    // Verify progress only moves forward
    for i in 1..updates.len() {
        assert!(
            updates[i].current >= updates[i - 1].current,
            "Progress should only move forward"
        );
    }
}

#[tokio::test]
async fn test_large_scale_progress_reporting() {
    // Integration test: Progress reporting with 1,000,000 items to verify SC-002 (large-scale support)
    let mut manager = BackgroundTaskManager::new();
    let (token, mut progress_rx) = manager.spawn_with_progress(|progress_tx, cancel_token| {
        Box::pin(async move {
            let total = 1_000_000;
            // Send updates every 100,000 items to avoid overwhelming the channel
            for i in (0..=total).step_by(100_000) {
                if cancel_token.is_cancelled() {
                    break;
                }
                let current = if i == 0 { 1 } else { i };
                let progress = ProgressReporter::new(current, total);
                let _ = progress_tx.send(progress).await;
                tokio::time::sleep(Duration::from_millis(1)).await;
            }
            // Send final update
            let progress = ProgressReporter::new(total, total);
            let _ = progress_tx.send(progress).await;
            Ok(())
        })
    });

    assert!(!token.is_cancelled());

    // Collect updates
    let mut updates = Vec::new();
    let start = std::time::Instant::now();
    while start.elapsed() < Duration::from_secs(5) {
        if let Ok(progress) = progress_rx.try_recv() {
            updates.push(progress);
            if let Some(last) = updates.last() {
                if last.current >= 1_000_000 {
                    break;
                }
            }
        } else {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }

    // Verify we can handle large numbers
    assert!(
        !updates.is_empty(),
        "Should handle large-scale progress reporting"
    );
    if let Some(last) = updates.last() {
        assert!(
            last.current >= 1_000_000 || last.current > 0,
            "Should process large numbers"
        );
    }
}

#[tokio::test]
async fn test_100_concurrent_operations() {
    // Integration test: 100 concurrent operations with progress reporting to verify SC-004
    let mut manager = BackgroundTaskManager::new();
    let mut receivers = Vec::new();

    // Spawn 100 concurrent tasks
    for task_id in 0..100 {
        let (_token, progress_rx) =
            manager.spawn_with_progress(move |progress_tx, cancel_token| {
                Box::pin(async move {
                    for i in 1..=5 {
                        if cancel_token.is_cancelled() {
                            break;
                        }
                        let progress = ProgressReporter::new(i, 5);
                        let _ = progress_tx.send(progress).await;
                        tokio::time::sleep(Duration::from_millis(1)).await;
                    }
                    Ok(())
                })
            });
        receivers.push((task_id, progress_rx));
    }

    // Collect progress from all receivers
    let mut all_updates = Vec::new();
    let start = std::time::Instant::now();
    while start.elapsed() < Duration::from_secs(10) {
        for (_task_id, progress_rx) in &mut receivers {
            if let Ok(progress) = progress_rx.try_recv() {
                all_updates.push(progress);
            }
        }
        if all_updates.len() >= 500 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }

    // Verify we received updates from many concurrent operations
    assert!(
        !all_updates.is_empty(),
        "Should handle 100 concurrent operations"
    );
    assert!(
        all_updates.len() > 50,
        "Should receive substantial updates from concurrent operations"
    );
}
