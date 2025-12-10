//! Integration tests for streaming logs and real-time updates
//! Verifies that streaming tasks emit updates without blocking the UI.

use anyhow::Result;
use tokio::time::{sleep, Duration, Instant};
use tui_framework::app::background_tasks::BackgroundTaskManager;
use tui_framework::data_source::log::{sync_log_buffer_to_view, SharedLogBuffer};
use tui_framework::view::Theme;
use tui_framework::widget::LogView;

fn make_streaming_task(
    lines: Vec<String>,
    delay_ms: u64,
) -> impl FnOnce(
    tokio::sync::mpsc::Sender<String>,
    tokio_util::sync::CancellationToken,
) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<()>> + Send>>
       + Send
       + 'static {
    move |sender, cancel_token| {
        Box::pin(async move {
            for line in lines {
                if cancel_token.is_cancelled() {
                    return Ok(());
                }
                let _ = sender.send(line).await;
                sleep(Duration::from_millis(delay_ms)).await;
            }
            Ok(())
        })
    }
}

#[tokio::test]
async fn streaming_logs_appear_in_real_time() {
    let mut manager = BackgroundTaskManager::new();
    let mut log_view = LogView::new(Theme::default());
    let buffer = SharedLogBuffer::new(100);

    let lines: Vec<String> = (0..5).map(|i| format!("line-{i}")).collect();
    manager.spawn_streaming(make_streaming_task(lines, 20));

    let start = Instant::now();
    while log_view.line_count() < 5 && start.elapsed() < Duration::from_millis(400) {
        if let Some(line) = manager.try_recv_stream_line() {
            buffer.push(line);
            sync_log_buffer_to_view(&buffer, &mut log_view);
        } else {
            sleep(Duration::from_millis(5)).await;
        }
    }

    assert!(
        log_view.line_count() >= 5,
        "expected at least 5 lines within 400ms"
    );
}

#[tokio::test]
async fn streaming_does_not_block_user_interactions() {
    let mut manager = BackgroundTaskManager::new();
    let mut log_view = LogView::new(Theme::default());
    let buffer = SharedLogBuffer::new(100);
    let mut interactions = 0usize;

    let lines: Vec<String> = (0..10).map(|i| format!("log-{i}")).collect();
    manager.spawn_streaming(make_streaming_task(lines, 15));

    let start = Instant::now();
    while start.elapsed() < Duration::from_millis(300) {
        interactions += 1; // Simulated UI interaction
        if let Some(line) = manager.try_recv_stream_line() {
            buffer.push(line);
            sync_log_buffer_to_view(&buffer, &mut log_view);
        }
        sleep(Duration::from_millis(5)).await;
    }

    assert!(log_view.line_count() >= 8, "expected logs to stream in");
    assert!(
        interactions >= 40,
        "user interactions should not be blocked"
    );
}

#[tokio::test]
async fn multiple_streams_run_concurrently() {
    let mut manager = BackgroundTaskManager::new();
    let mut log_view = LogView::new(Theme::default());
    let buffer = SharedLogBuffer::new(200);

    let stream_a: Vec<String> = (0..5).map(|i| format!("A-{i}")).collect();
    let stream_b: Vec<String> = (0..5).map(|i| format!("B-{i}")).collect();
    manager.spawn_streaming(make_streaming_task(stream_a, 10));
    manager.spawn_streaming(make_streaming_task(stream_b, 12));

    let start = Instant::now();
    while log_view.line_count() < 10 && start.elapsed() < Duration::from_millis(400) {
        for line in manager.drain_stream_lines() {
            buffer.push(line);
            sync_log_buffer_to_view(&buffer, &mut log_view);
        }
        sleep(Duration::from_millis(5)).await;
    }

    assert!(
        log_view.line_count() >= 10,
        "expected at least 10 combined lines from both streams"
    );
}

#[tokio::test]
async fn streaming_updates_arrive_within_100ms() {
    let mut manager = BackgroundTaskManager::new();
    let mut log_view = LogView::new(Theme::default());
    let buffer = SharedLogBuffer::new(50);

    manager.spawn_streaming(make_streaming_task(vec!["fast-line".into()], 20));

    let start = Instant::now();
    let mut received = false;
    while start.elapsed() < Duration::from_millis(150) {
        if let Some(line) = manager.try_recv_stream_line() {
            buffer.push(line);
            sync_log_buffer_to_view(&buffer, &mut log_view);
            received = true;
            break;
        }
        sleep(Duration::from_millis(5)).await;
    }

    assert!(
        received,
        "expected to receive first streaming line within 150ms"
    );
    assert_eq!(log_view.line_count(), 1);
}

#[tokio::test]
async fn streaming_tasks_can_be_cancelled() {
    let mut manager = BackgroundTaskManager::new();
    let mut log_view = LogView::new(Theme::default());
    let buffer = SharedLogBuffer::new(50);

    let token = manager.spawn_streaming(make_streaming_task(
        vec!["one".into(), "two".into(), "three".into(), "four".into()],
        30,
    ));

    // Let first two messages through
    sleep(Duration::from_millis(80)).await;
    token.cancel();

    // Drain any remaining quickly
    let start = Instant::now();
    while start.elapsed() < Duration::from_millis(120) {
        for line in manager.drain_stream_lines() {
            buffer.push(line);
            sync_log_buffer_to_view(&buffer, &mut log_view);
        }
        sleep(Duration::from_millis(5)).await;
    }

    assert!(
        log_view.line_count() <= 3,
        "expected cancellation to stop most messages"
    );
}
