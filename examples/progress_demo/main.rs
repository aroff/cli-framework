//! Example demonstrating progress reporting features

use std::time::Duration;
use tui_framework::app::background_tasks::{BackgroundTaskManager, ProgressReporter};
use tui_framework::progress_formatting;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let mut manager = BackgroundTaskManager::new();

    // Example 1: Basic progress reporting
    println!("Example 1: Basic progress reporting");
    let (_token, mut progress_rx) = manager.spawn_with_progress(|progress_tx, cancel_token| {
        Box::pin(async move {
            let total = 10;
            for i in 1..=total {
                if cancel_token.is_cancelled() {
                    break;
                }
                let progress = ProgressReporter::new(i, total);
                let _ = progress_tx.send(progress).await;
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
            Ok(())
        })
    });

    while let Ok(progress) = progress_rx.try_recv() {
        println!(
            "Progress: {}/{} ({}%)",
            progress.current,
            progress.total.unwrap_or(0),
            progress.percentage()
        );
    }

    // Example 2: Progress with contextual messages
    println!("\nExample 2: Progress with contextual messages");
    let (_token2, mut progress_rx2) = manager.spawn_with_progress(|progress_tx, cancel_token| {
        Box::pin(async move {
            let files = ["file1.jpg", "file2.jpg", "file3.jpg"];
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
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
            Ok(())
        })
    });

    while let Ok(progress) = progress_rx2.try_recv() {
        if let Some(ref msg) = progress.message {
            println!(
                "Progress: {}/{} - {}",
                progress.current,
                progress.total.unwrap_or(0),
                msg
            );
        }
    }

    // Example 3: Concurrent operations
    println!("\nExample 3: Concurrent operations");
    let mut manager3 = BackgroundTaskManager::new();
    let mut receivers = Vec::new();

    for task_id in 0..3 {
        let (_token, progress_rx) =
            manager3.spawn_with_progress(move |progress_tx, cancel_token| {
                let task_id = task_id;
                Box::pin(async move {
                    for i in 1..=5 {
                        if cancel_token.is_cancelled() {
                            break;
                        }
                        let progress = ProgressReporter::with_message(
                            i,
                            5,
                            format!("Task {}: item {}", task_id, i),
                        );
                        let _ = progress_tx.send(progress).await;
                        tokio::time::sleep(Duration::from_millis(50)).await;
                    }
                    Ok(())
                })
            });
        receivers.push((task_id, progress_rx));
    }

    // Poll all receivers
    let mut last_displayed = [0; 3];
    loop {
        let mut all_done = true;
        for (task_id, progress_rx) in &mut receivers {
            while let Ok(progress) = progress_rx.try_recv() {
                if progress_formatting::should_display_progress(
                    progress.current,
                    last_displayed[*task_id as usize],
                ) {
                    if let Some(ref msg) = progress.message {
                        println!(
                            "Task {}: {}/{} - {}",
                            task_id,
                            progress.current,
                            progress.total.unwrap_or(0),
                            msg
                        );
                    }
                    last_displayed[*task_id as usize] = progress.current;
                }
                if progress.current < 5 {
                    all_done = false;
                }
            }
        }
        if all_done {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }

    Ok(())
}
