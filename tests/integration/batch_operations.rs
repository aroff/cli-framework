//! Integration tests for batch task operations

use std::time::Duration;
use tui_framework::app::background_tasks::{
    task_definition, BackgroundTaskManager,
};
use tokio::time::sleep;

#[tokio::test]
async fn test_parallel_file_processing_scenario() {
    let mut manager = BackgroundTaskManager::new();

    // Simulate processing 20 files with varying durations
    let tasks: Vec<_> = (0..20)
        .map(|i| {
            task_definition(
                move || {
                    let file_num = i;
                    async move {
                        // Simulate file processing time (10-50ms per file)
                        let delay_ms = 10 + (file_num % 5) * 10;
                        sleep(Duration::from_millis(delay_ms)).await;
                        Ok(format!("processed-file-{}", file_num))
                    }
                },
                Some(format!("file-{}", i)),
            )
        })
        .collect();

    let start = std::time::Instant::now();
    let result = manager.spawn_batch(tasks, Some(5)).await; // Limit to 5 concurrent
    let elapsed = start.elapsed();

    // Verify all files processed
    assert_eq!(result.total, 20);
    assert_eq!(result.successful, 20);
    assert_eq!(result.failed, 0);
    assert!(result.all_succeeded());

    // Verify results contain all files
    let processed_files: Vec<String> = result
        .results
        .iter()
        .filter_map(|r| r.value::<String>().cloned())
        .collect();
    assert_eq!(processed_files.len(), 20);

    // With concurrency limit of 5, processing 20 files should take at least
    // 4 batches * ~50ms = ~200ms, but allow some variance
    assert!(elapsed.as_millis() >= 150); // Allow some variance but should be significantly longer than single batch
}

#[tokio::test]
async fn test_batch_api_operations_scenario() {
    let mut manager = BackgroundTaskManager::new();

    // Simulate 50 API operations with varying durations
    let tasks: Vec<_> = (0..50)
        .map(|i| {
            task_definition(
                move || {
                    let op_num = i;
                    async move {
                        // Simulate API call time (20-100ms per operation)
                        let delay_ms = 20 + (op_num % 5) * 20;
                        sleep(Duration::from_millis(delay_ms)).await;
                        Ok(format!("api-result-{}", op_num))
                    }
                },
                Some(format!("api-op-{}", i)),
            )
        })
        .collect();

    let start = std::time::Instant::now();
    let result = manager.spawn_batch(tasks, Some(10)).await; // Limit to 10 concurrent
    let elapsed = start.elapsed();

    // Verify all operations completed
    assert_eq!(result.total, 50);
    assert_eq!(result.successful, 50);
    assert_eq!(result.failed, 0);
    assert!(result.all_succeeded());

    // With concurrency limit of 10, processing 50 operations should take
    // at least 5 batches * ~100ms = ~500ms
    assert!(elapsed.as_millis() >= 400); // Allow some variance
}

#[tokio::test]
async fn test_concurrent_data_processing_scenario() {
    let mut manager = BackgroundTaskManager::new();

    // Simulate processing 100 data chunks with varying durations
    let tasks: Vec<_> = (0..100)
        .map(|i| {
            task_definition(
                move || {
                    let chunk_num = i;
                    async move {
                        // Simulate chunk processing time (5-25ms per chunk)
                        let delay_ms = 5 + (chunk_num % 5) * 5;
                        sleep(Duration::from_millis(delay_ms)).await;
                        Ok(format!("processed-chunk-{}", chunk_num))
                    }
                },
                Some(format!("chunk-{}", i)),
            )
        })
        .collect();

    let start = std::time::Instant::now();
    let result = manager.spawn_batch(tasks, Some(20)).await; // Limit to 20 concurrent
    let elapsed = start.elapsed();

    // Verify all chunks processed
    assert_eq!(result.total, 100);
    assert_eq!(result.successful, 100);
    assert_eq!(result.failed, 0);
    assert_eq!(result.cancelled, 0);
    assert!(result.all_succeeded());

    // With concurrency limit of 20, processing 100 chunks should take
    // at least 5 batches * ~25ms = ~125ms
    assert!(elapsed.as_millis() >= 100); // Allow some variance
}
