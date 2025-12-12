//! Unit tests for background task management, including batch operations

use tui_framework::app::background_tasks::{
    task_definition, BackgroundTaskManager, BatchResult, TaskStatus,
};

#[tokio::test]
async fn test_basic_batch_spawning() {
    let mut manager = BackgroundTaskManager::new();

    // Create 10 tasks that return their index
    let tasks: Vec<_> = (0..10)
        .map(|i| {
            task_definition(
                move || {
                    let idx = i;
                    async move { Ok(idx) }
                },
                Some(format!("task-{}", i)),
            )
        })
        .collect();

    let result = manager.spawn_batch(tasks, None).await;

    // Verify all tasks completed
    assert_eq!(result.total, 10);
    assert_eq!(result.successful, 10);
    assert_eq!(result.failed, 0);
    assert_eq!(result.cancelled, 0);
    assert_eq!(result.results.len(), 10);
    assert!(result.all_succeeded());
    assert!(!result.has_failures());

    // Verify results are in completion order (not spawn order)
    // Since tasks complete quickly, order may vary
    let values: Vec<i32> = result
        .results
        .iter()
        .filter_map(|r| r.value::<i32>().copied())
        .collect();
    assert_eq!(values.len(), 10);
    // Verify all expected values are present
    let mut sorted_values = values.clone();
    sorted_values.sort();
    assert_eq!(sorted_values, (0..10).collect::<Vec<_>>());
}

#[tokio::test]
async fn test_empty_batch_handling() {
    let mut manager = BackgroundTaskManager::new();

    let result = manager.spawn_batch(vec![], None).await;

    // Verify empty result
    assert_eq!(result.total, 0);
    assert_eq!(result.successful, 0);
    assert_eq!(result.failed, 0);
    assert_eq!(result.cancelled, 0);
    assert_eq!(result.success_rate, 0.0);
    assert_eq!(result.results.len(), 0);
    assert_eq!(result.errors.len(), 0);
    assert!(result.all_succeeded()); // Empty batch is considered successful
}

#[tokio::test]
async fn test_batch_with_failures() {
    let mut manager = BackgroundTaskManager::new();

    let tasks = vec![
        task_definition(|| async { Ok(1) }, Some("task-1".to_string())),
        task_definition(
            || async { Err(anyhow::anyhow!("Task failed")) },
            Some("task-2".to_string()),
        ),
        task_definition(|| async { Ok(3) }, Some("task-3".to_string())),
    ];

    let result = manager.spawn_batch(tasks, None).await;

    assert_eq!(result.total, 3);
    assert_eq!(result.successful, 2);
    assert_eq!(result.failed, 1);
    assert_eq!(result.cancelled, 0);
    assert!(!result.all_succeeded());
    assert!(result.has_failures());
    assert_eq!(result.errors.len(), 1);

    // Verify error message includes task identifier
    let error_msg = result.errors[0].1.to_string();
    assert!(error_msg.contains("task-2") || error_msg.contains("Task failed"));
}

#[tokio::test]
async fn test_concurrency_limiting() {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::Duration;
    use tokio::time::sleep;

    let mut manager = BackgroundTaskManager::new();
    let concurrent_count = Arc::new(AtomicUsize::new(0));
    let max_concurrent = Arc::new(AtomicUsize::new(0));

    // Create 20 tasks that track concurrent execution
    let tasks: Vec<_> = (0..20)
        .map(|i| {
            let concurrent_count = concurrent_count.clone();
            let max_concurrent = max_concurrent.clone();
            task_definition(
                move || {
                    let concurrent_count = concurrent_count.clone();
                    let max_concurrent = max_concurrent.clone();
                    async move {
                        let current = concurrent_count.fetch_add(1, Ordering::SeqCst) + 1;
                        let max = max_concurrent.load(Ordering::SeqCst);
                        if current > max {
                            max_concurrent.store(current, Ordering::SeqCst);
                        }
                        sleep(Duration::from_millis(50)).await; // Simulate work
                        concurrent_count.fetch_sub(1, Ordering::SeqCst);
                        Ok(i)
                    }
                },
                Some(format!("task-{}", i)),
            )
        })
        .collect();

    let result = manager.spawn_batch(tasks, Some(5)).await; // Limit to 5 concurrent

    // Verify all tasks completed
    assert_eq!(result.total, 20);
    assert_eq!(result.successful, 20);

    // Verify concurrency was limited (max should be <= 5, allowing for some variance)
    let max = max_concurrent.load(Ordering::SeqCst);
    assert!(max <= 5, "Max concurrent tasks ({}) exceeded limit (5)", max);
}

#[tokio::test]
async fn test_default_limit_calculation() {
    let mut manager = BackgroundTaskManager::new();

    // Test that default limit is used when None is provided
    let tasks: Vec<_> = (0..10)
        .map(|i| {
            task_definition(
                move || async move { Ok(i) },
                Some(format!("task-{}", i)),
            )
        })
        .collect();

    let result = manager.spawn_batch(tasks, None).await; // Use default limit

    assert_eq!(result.total, 10);
    assert_eq!(result.successful, 10);
}

#[tokio::test]
async fn test_maximum_limit_enforcement() {
    let mut manager = BackgroundTaskManager::new();

    // Request a limit higher than the maximum (100)
    let tasks: Vec<_> = (0..10)
        .map(|i| {
            task_definition(
                move || async move { Ok(i) },
                Some(format!("task-{}", i)),
            )
        })
        .collect();

    let result = manager.spawn_batch(tasks, Some(200)).await; // Request 200, should be capped at 100

    // Should still work, just with capped limit
    assert_eq!(result.total, 10);
    assert_eq!(result.successful, 10);
}

#[tokio::test]
async fn test_sequential_execution() {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::Duration;
    use tokio::time::sleep;

    let mut manager = BackgroundTaskManager::new();
    let max_concurrent = Arc::new(AtomicUsize::new(0));

    // Create 5 tasks that track execution order
    let tasks: Vec<_> = (0..5)
        .map(|i| {
            let max_concurrent = max_concurrent.clone();
            task_definition(
                move || {
                    let max_concurrent = max_concurrent.clone();
                    async move {
                        let current = max_concurrent.fetch_add(1, Ordering::SeqCst) + 1;
                        let max = max_concurrent.load(Ordering::SeqCst);
                        if current > max {
                            max_concurrent.store(current, Ordering::SeqCst);
                        }
                        sleep(Duration::from_millis(10)).await;
                        max_concurrent.fetch_sub(1, Ordering::SeqCst);
                        Ok(i)
                    }
                },
                Some(format!("task-{}", i)),
            )
        })
        .collect();

    let result = manager.spawn_batch(tasks, Some(1)).await; // Limit of 1 = sequential

    assert_eq!(result.total, 5);
    assert_eq!(result.successful, 5);

    // With limit of 1, max concurrent should be 1
    let max = max_concurrent.load(Ordering::SeqCst);
    assert_eq!(max, 1, "Sequential execution should have max concurrent = 1");
}

#[tokio::test]
async fn test_wait_for_all() {
    use std::time::Duration;
    use tokio::time::sleep;

    let mut manager = BackgroundTaskManager::new();

    // Spawn some individual tasks
    for i in 0..5 {
        manager.spawn(async move {
            sleep(Duration::from_millis(10 * (i + 1) as u64)).await;
            Ok(())
        });
    }

    // Wait for all tasks to complete
    let result = manager.wait_for_all().await;

    assert_eq!(result.total, 5);
    assert_eq!(result.successful, 5);
    assert_eq!(result.failed, 0);
    assert_eq!(result.cancelled, 0);
    assert!(result.all_succeeded());
}

#[tokio::test]
async fn test_wait_for_all_empty() {
    let mut manager = BackgroundTaskManager::new();

    let result = manager.wait_for_all().await;

    assert_eq!(result.total, 0);
    assert_eq!(result.successful, 0);
    assert_eq!(result.failed, 0);
    assert_eq!(result.cancelled, 0);
}

#[tokio::test]
async fn test_cancelled_task_accounting() {
    use std::time::Duration;
    use tokio::time::sleep;

    let mut manager = BackgroundTaskManager::new();

    // Create a batch with some tasks that will be cancelled
    let tasks: Vec<_> = (0..10)
        .map(|i| {
            task_definition(
                move || {
                    let delay = i;
                    async move {
                        sleep(Duration::from_millis(100 + delay * 10)).await;
                        Ok(i)
                    }
                },
                Some(format!("task-{}", i)),
            )
        })
        .collect();

    // Spawn batch but don't wait - this simulates cancellation scenario
    // For actual cancellation testing, we'd need spawn_batch to return a token
    // For now, test that cancelled count is tracked correctly in results
    let result = manager.spawn_batch(tasks, Some(5)).await;

    // All tasks should complete (no actual cancellation in this test)
    assert_eq!(result.total, 10);
    // Verify cancelled count field exists and works
    assert_eq!(result.cancelled, 0);
    
    // Test success rate calculation excludes cancelled
    if result.successful + result.failed > 0 {
        let expected_rate = (result.successful as f64 / (result.successful + result.failed) as f64) * 100.0;
        assert!((result.success_rate - expected_rate).abs() < 0.01);
    }
}

#[tokio::test]
async fn test_single_task_batch() {
    let mut manager = BackgroundTaskManager::new();

    let tasks = vec![task_definition(
        || async { Ok(42) },
        Some("single-task".to_string()),
    )];

    let result = manager.spawn_batch(tasks, None).await;

    assert_eq!(result.total, 1);
    assert_eq!(result.successful, 1);
    assert_eq!(result.failed, 0);
    assert_eq!(result.cancelled, 0);
    assert!(result.all_succeeded());
    
    let value = result.results[0].value::<i32>();
    assert_eq!(value, Some(&42));
}

#[tokio::test]
async fn test_very_large_batch() {
    let mut manager = BackgroundTaskManager::new();

    // Create 1000 tasks
    let tasks: Vec<_> = (0..1000)
        .map(|i| {
            task_definition(
                move || async move { Ok(i) },
                None, // No identifier to test index fallback
            )
        })
        .collect();

    let result = manager.spawn_batch(tasks, Some(50)).await; // Limit to 50 concurrent

    assert_eq!(result.total, 1000);
    assert_eq!(result.successful, 1000);
    assert_eq!(result.failed, 0);
    assert!(result.all_succeeded());
    
    // Verify all values are present
    let values: Vec<i32> = result
        .results
        .iter()
        .filter_map(|r| r.value::<i32>().copied())
        .collect();
    assert_eq!(values.len(), 1000);
}

#[tokio::test]
async fn test_mixed_success_failure() {
    let mut manager = BackgroundTaskManager::new();

    let tasks = vec![
        task_definition(|| async { Ok(1) }, Some("task-1".to_string())),
        task_definition(|| async { Err(anyhow::anyhow!("Error 1")) }, Some("task-2".to_string())),
        task_definition(|| async { Ok(3) }, Some("task-3".to_string())),
        task_definition(|| async { Err(anyhow::anyhow!("Error 2")) }, Some("task-4".to_string())),
        task_definition(|| async { Ok(5) }, Some("task-5".to_string())),
    ];

    let result = manager.spawn_batch(tasks, None).await;

    assert_eq!(result.total, 5);
    assert_eq!(result.successful, 3);
    assert_eq!(result.failed, 2);
    assert_eq!(result.cancelled, 0);
    assert!(!result.all_succeeded());
    assert!(result.has_failures());
    assert_eq!(result.errors.len(), 2);
    
    // Verify success rate excludes cancelled tasks
    let expected_rate = (3.0 / 5.0) * 100.0;
    assert!((result.success_rate - expected_rate).abs() < 0.01);
}

#[tokio::test]
async fn test_all_tasks_fail() {
    let mut manager = BackgroundTaskManager::new();

    let tasks: Vec<_> = (0..5)
        .map(|i| {
            task_definition(
                move || async { Err(anyhow::anyhow!("Task {} failed", i)) },
                Some(format!("task-{}", i)),
            )
        })
        .collect();

    let result = manager.spawn_batch(tasks, None).await;

    assert_eq!(result.total, 5);
    assert_eq!(result.successful, 0);
    assert_eq!(result.failed, 5);
    assert_eq!(result.cancelled, 0);
    assert!(!result.all_succeeded());
    assert!(result.has_failures());
    assert_eq!(result.errors.len(), 5);
    assert_eq!(result.success_rate, 0.0);
}

#[tokio::test]
async fn test_backward_compatibility_spawn() {
    let mut manager = BackgroundTaskManager::new();

    // Test that existing spawn() method still works
    let token = manager.spawn(async { Ok(()) });
    
    // Wait a bit for task to complete
    tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    
    // Try to receive result
    let result = manager.try_recv_result();
    assert!(result.is_some());
    
    // Clean up
    manager.cancel_task(&token);
}

#[test]
fn test_format_errors_provided_identifiers() {
    use tui_framework::app::background_tasks::{BatchResult, TaskIdentifier};
    use anyhow::anyhow;
    
    let result = BatchResult {
        total: 3,
        successful: 0,
        failed: 3,
        cancelled: 0,
        success_rate: 0.0,
        errors: vec![
            (TaskIdentifier::Provided("file: image.jpg".to_string()), anyhow!("permission denied")),
            (TaskIdentifier::Provided("file: data.json".to_string()), anyhow!("invalid JSON")),
            (TaskIdentifier::Provided("file: script.sh".to_string()), anyhow!("execution failed")),
        ],
        results: Vec::new(),
        filtered_errors: Vec::new(),
        truncated: false,
        custom_summary: None,
    };
    
    let formatted = result.format_errors();
    assert!(formatted.contains("Errors (3)"));
    assert!(formatted.contains("[file: image.jpg]"));
    assert!(formatted.contains("[file: data.json]"));
    assert!(formatted.contains("[file: script.sh]"));
    assert!(formatted.contains("permission denied"));
}

#[test]
fn test_format_errors_positional_index() {
    use tui_framework::app::background_tasks::{BatchResult, TaskIdentifier};
    use anyhow::anyhow;
    
    let result = BatchResult {
        total: 2,
        successful: 0,
        failed: 2,
        cancelled: 0,
        success_rate: 0.0,
        errors: vec![
            (TaskIdentifier::Index(0), anyhow!("Network timeout")),
            (TaskIdentifier::Index(5), anyhow!("Connection refused")),
        ],
        results: Vec::new(),
        filtered_errors: Vec::new(),
        truncated: false,
        custom_summary: None,
    };
    
    let formatted = result.format_errors();
    assert!(formatted.contains("Errors (2)"));
    assert!(formatted.contains("[Task 0]"));
    assert!(formatted.contains("[Task 5]"));
    assert!(formatted.contains("Network timeout"));
}

#[test]
fn test_format_errors_empty() {
    use tui_framework::app::background_tasks::{BatchResult, TaskIdentifier};
    
    let result = BatchResult {
        total: 5,
        successful: 5,
        failed: 0,
        cancelled: 0,
        success_rate: 100.0,
        errors: Vec::new(),
        results: Vec::new(),
        filtered_errors: Vec::new(),
        truncated: false,
        custom_summary: None,
    };
    
    let formatted = result.format_errors();
    assert_eq!(formatted, "");
}

#[test]
fn test_with_summary() {
    use tui_framework::app::background_tasks::{BatchResult, TaskIdentifier};
    
    let result = BatchResult {
        total: 10,
        successful: 8,
        failed: 2,
        cancelled: 0,
        success_rate: 80.0,
        errors: Vec::new(),
        results: Vec::new(),
        filtered_errors: Vec::new(),
        truncated: false,
        custom_summary: None,
    };
    
    let custom = result.with_summary("Custom summary message");
    assert_eq!(custom.custom_summary, Some("Custom summary message".to_string()));
    assert_eq!(custom.generate_summary(), "Custom summary message");
}

#[test]
fn test_aggregate_results() {
    use tui_framework::app::background_tasks::{aggregate_results, BatchTaskResult, TaskIdentifier, TaskStatus};
    use anyhow::anyhow;
    
    let results = vec![
        BatchTaskResult {
            identifier: TaskIdentifier::Provided("task1".to_string()),
            status: TaskStatus::Success,
            value: Some(Box::new(1)),
            error: None,
        },
        BatchTaskResult {
            identifier: TaskIdentifier::Provided("task2".to_string()),
            status: TaskStatus::Failure,
            value: None,
            error: Some(anyhow!("Error 2")),
        },
        BatchTaskResult {
            identifier: TaskIdentifier::Provided("task3".to_string()),
            status: TaskStatus::Success,
            value: Some(Box::new(3)),
            error: None,
        },
        BatchTaskResult {
            identifier: TaskIdentifier::Provided("task4".to_string()),
            status: TaskStatus::Cancelled,
            value: None,
            error: None,
        },
    ];
    
    let aggregated = aggregate_results(results);
    
    assert_eq!(aggregated.total, 4);
    assert_eq!(aggregated.successful, 2);
    assert_eq!(aggregated.failed, 1);
    assert_eq!(aggregated.cancelled, 1);
    assert_eq!(aggregated.errors.len(), 1);
    assert!((aggregated.success_rate - 66.67).abs() < 0.1);
}

#[test]
fn test_merge_results() {
    use tui_framework::app::background_tasks::{merge_results, BatchResult, TaskIdentifier};
    use anyhow::anyhow;
    
    let result1 = BatchResult {
        total: 10,
        successful: 8,
        failed: 2,
        cancelled: 0,
        success_rate: 80.0,
        errors: vec![
            (TaskIdentifier::Index(0), anyhow!("Error 1")),
            (TaskIdentifier::Index(1), anyhow!("Error 2")),
        ],
        results: Vec::new(),
        filtered_errors: Vec::new(),
        truncated: false,
        custom_summary: None,
    };
    
    let result2 = BatchResult {
        total: 5,
        successful: 4,
        failed: 1,
        cancelled: 0,
        success_rate: 80.0,
        errors: vec![
            (TaskIdentifier::Index(0), anyhow!("Error 3")),
        ],
        results: Vec::new(),
        filtered_errors: Vec::new(),
        truncated: false,
        custom_summary: None,
    };
    
    let merged = merge_results(&[result1, result2]);
    
    assert_eq!(merged.total, 15);
    assert_eq!(merged.successful, 12);
    assert_eq!(merged.failed, 3);
    assert_eq!(merged.errors.len(), 3);
    assert!((merged.success_rate - 80.0).abs() < 0.1);
}

#[test]
fn test_merge_results_empty() {
    use tui_framework::app::background_tasks::merge_results;
    
    let merged = merge_results(&[]);
    
    assert_eq!(merged.total, 0);
    assert_eq!(merged.successful, 0);
    assert_eq!(merged.failed, 0);
    assert_eq!(merged.errors.len(), 0);
}

#[test]
fn test_aggregate_with_filter() {
    use tui_framework::app::background_tasks::{aggregate_with_filter, BatchTaskResult, TaskIdentifier, TaskStatus};
    use anyhow::anyhow;
    
    let results = vec![
        BatchTaskResult {
            identifier: TaskIdentifier::Index(0),
            status: TaskStatus::Success,
            value: None,
            error: None,
        },
        BatchTaskResult {
            identifier: TaskIdentifier::Index(1),
            status: TaskStatus::Failure,
            value: None,
            error: Some(anyhow!("Network error")),
        },
        BatchTaskResult {
            identifier: TaskIdentifier::Index(2),
            status: TaskStatus::Failure,
            value: None,
            error: Some(anyhow!("Timeout")),
        },
    ];
    
    // Filter out "Timeout" errors
    let filtered = aggregate_with_filter(results, |err| {
        !err.to_string().contains("Timeout")
    });
    
    assert_eq!(filtered.total, 3);
    assert_eq!(filtered.successful, 1);
    assert_eq!(filtered.failed, 1); // Only Network error counted
    assert_eq!(filtered.errors.len(), 1);
    assert_eq!(filtered.filtered_errors.len(), 1); // Timeout filtered
}

#[test]
fn test_aggregate_with_limit() {
    use tui_framework::app::background_tasks::{aggregate_with_limit, BatchTaskResult, TaskIdentifier, TaskStatus};
    use anyhow::anyhow;
    
    let results: Vec<_> = (0..10)
        .map(|i| BatchTaskResult {
            identifier: TaskIdentifier::Index(i),
            status: TaskStatus::Failure,
            value: None,
            error: Some(anyhow!("Error {}", i)),
        })
        .collect();
    
    let limited = aggregate_with_limit(results, Some(3));
    
    assert_eq!(limited.total, 10);
    assert_eq!(limited.failed, 10);
    assert_eq!(limited.errors.len(), 3); // Limited to 3
    assert!(limited.truncated); // Should be truncated
}

#[test]
fn test_aggregate_with_limit_no_limit() {
    use tui_framework::app::background_tasks::{aggregate_with_limit, BatchTaskResult, TaskIdentifier, TaskStatus};
    use anyhow::anyhow;
    
    let results: Vec<_> = (0..5)
        .map(|i| BatchTaskResult {
            identifier: TaskIdentifier::Index(i),
            status: TaskStatus::Failure,
            value: None,
            error: Some(anyhow!("Error {}", i)),
        })
        .collect();
    
    let unlimited = aggregate_with_limit(results, None);
    
    assert_eq!(unlimited.total, 5);
    assert_eq!(unlimited.errors.len(), 5); // All errors collected
    assert!(!unlimited.truncated);
}

#[test]
fn test_edge_case_empty_results() {
    use tui_framework::app::background_tasks::{aggregate_results, BatchTaskResult, TaskIdentifier, TaskStatus};
    
    let result = aggregate_results(Vec::new());
    
    assert_eq!(result.total, 0);
    assert_eq!(result.successful, 0);
    assert_eq!(result.failed, 0);
    assert_eq!(result.cancelled, 0);
    assert_eq!(result.success_rate, 0.0);
    assert_eq!(result.errors.len(), 0);
}

#[test]
fn test_edge_case_all_cancelled() {
    use tui_framework::app::background_tasks::{aggregate_results, BatchTaskResult, TaskIdentifier, TaskStatus};
    
    let results: Vec<_> = (0..5)
        .map(|i| BatchTaskResult {
            identifier: TaskIdentifier::Index(i),
            status: TaskStatus::Cancelled,
            value: None,
            error: None,
        })
        .collect();
    
    let result = aggregate_results(results);
    
    assert_eq!(result.total, 5);
    assert_eq!(result.successful, 0);
    assert_eq!(result.failed, 0);
    assert_eq!(result.cancelled, 5);
    // Success rate should be 0 when no successful or failed tasks
    assert_eq!(result.success_rate, 0.0);
}

#[test]
fn test_edge_case_zero_total() {
    use tui_framework::app::background_tasks::{BatchResult, TaskIdentifier};
    
    let result = BatchResult {
        total: 0,
        successful: 0,
        failed: 0,
        cancelled: 0,
        success_rate: 0.0,
        errors: Vec::new(),
        results: Vec::new(),
        filtered_errors: Vec::new(),
        truncated: false,
        custom_summary: None,
    };
    
    let summary = result.generate_summary();
    assert!(summary.contains("No tasks executed"));
    
    let errors = result.format_errors();
    assert_eq!(errors, "");
    
    assert!(result.all_succeeded()); // Empty batch is considered successful
}

#[test]
fn test_performance_summary_generation() {
    use tui_framework::app::background_tasks::{BatchResult, TaskIdentifier};
    use std::time::Instant;
    
    // Create a large batch result (10,000 tasks)
    let result = BatchResult {
        total: 10_000,
        successful: 9_500,
        failed: 500,
        cancelled: 0,
        success_rate: 95.0,
        errors: Vec::new(), // Empty errors for performance test
        results: Vec::new(),
        filtered_errors: Vec::new(),
        truncated: false,
        custom_summary: None,
    };
    
    let start = Instant::now();
    let _summary = result.generate_summary();
    let duration = start.elapsed();
    
    // Should complete in under 10ms (SC-005)
    assert!(duration.as_millis() < 10, "Summary generation took {}ms, expected < 10ms", duration.as_millis());
}

#[test]
fn test_performance_error_formatting() {
    use tui_framework::app::background_tasks::{BatchResult, TaskIdentifier};
    use anyhow::anyhow;
    use std::time::Instant;
    
    // Create result with many errors (1,000 errors)
    let errors: Vec<_> = (0..1_000)
        .map(|i| (TaskIdentifier::Index(i), anyhow!("Error {}", i)))
        .collect();
    
    let result = BatchResult {
        total: 1_000,
        successful: 0,
        failed: 1_000,
        cancelled: 0,
        success_rate: 0.0,
        errors,
        results: Vec::new(),
        filtered_errors: Vec::new(),
        truncated: false,
        custom_summary: None,
    };
    
    let start = Instant::now();
    let _formatted = result.format_errors();
    let duration = start.elapsed();
    
    // Should complete reasonably quickly (linear time complexity)
    assert!(duration.as_millis() < 100, "Error formatting took {}ms", duration.as_millis());
}

// ============================================================================
// Task Result Aggregation Tests
// ============================================================================

#[test]
fn test_generate_summary_all_success() {
    use tui_framework::app::background_tasks::{BatchResult, TaskIdentifier};
    
    let result = BatchResult {
        total: 45,
        successful: 45,
        failed: 0,
        cancelled: 0,
        success_rate: 100.0,
        errors: Vec::new(),
        results: Vec::new(),
        filtered_errors: Vec::new(),
        truncated: false,
        custom_summary: None,
    };
    
    let summary = result.generate_summary();
    assert!(summary.contains("All 45 tasks completed successfully") || summary.contains("45"));
    assert!(summary.contains("successfully"));
}

#[test]
fn test_generate_summary_mixed_results() {
    use tui_framework::app::background_tasks::{BatchResult, TaskIdentifier};
    use anyhow::anyhow;
    
    let result = BatchResult {
        total: 45,
        successful: 42,
        failed: 3,
        cancelled: 0,
        success_rate: 93.33,
        errors: vec![
            (TaskIdentifier::Provided("file1".to_string()), anyhow!("Error 1")),
            (TaskIdentifier::Provided("file2".to_string()), anyhow!("Error 2")),
            (TaskIdentifier::Provided("file3".to_string()), anyhow!("Error 3")),
        ],
        results: Vec::new(),
        filtered_errors: Vec::new(),
        truncated: false,
        custom_summary: None,
    };
    
    let summary = result.generate_summary();
    assert!(summary.contains("45"));
    assert!(summary.contains("42"));
    assert!(summary.contains("3"));
    assert!(summary.contains("93") || summary.contains("success rate"));
}

#[test]
fn test_generate_summary_all_failure() {
    use tui_framework::app::background_tasks::{BatchResult, TaskIdentifier};
    use anyhow::anyhow;
    
    let result = BatchResult {
        total: 5,
        successful: 0,
        failed: 5,
        cancelled: 0,
        success_rate: 0.0,
        errors: vec![
            (TaskIdentifier::Index(0), anyhow!("Error 1")),
            (TaskIdentifier::Index(1), anyhow!("Error 2")),
            (TaskIdentifier::Index(2), anyhow!("Error 3")),
            (TaskIdentifier::Index(3), anyhow!("Error 4")),
            (TaskIdentifier::Index(4), anyhow!("Error 5")),
        ],
        results: Vec::new(),
        filtered_errors: Vec::new(),
        truncated: false,
        custom_summary: None,
    };
    
    let summary = result.generate_summary();
    assert!(summary.contains("5"));
    assert!(summary.contains("0") || summary.contains("failed"));
    assert!(summary.contains("0%") || summary.contains("success rate"));
}

#[test]
fn test_generate_summary_empty_results() {
    use tui_framework::app::background_tasks::{BatchResult, TaskIdentifier};
    
    let result = BatchResult {
        total: 0,
        successful: 0,
        failed: 0,
        cancelled: 0,
        success_rate: 0.0,
        errors: Vec::new(),
        results: Vec::new(),
        filtered_errors: Vec::new(),
        truncated: false,
        custom_summary: None,
    };
    
    let summary = result.generate_summary();
    assert!(summary.contains("No tasks executed") || summary.contains("0"));
}

#[test]
fn test_format_errors_provided_identifiers() {
    use tui_framework::app::background_tasks::{BatchResult, TaskIdentifier};
    use anyhow::anyhow;
    
    let result = BatchResult {
        total: 3,
        successful: 0,
        failed: 3,
        cancelled: 0,
        success_rate: 0.0,
        errors: vec![
            (TaskIdentifier::Provided("file: image.jpg".to_string()), anyhow!("permission denied")),
            (TaskIdentifier::Provided("file: data.json".to_string()), anyhow!("invalid JSON")),
            (TaskIdentifier::Provided("file: script.sh".to_string()), anyhow!("execution failed")),
        ],
        results: Vec::new(),
        filtered_errors: Vec::new(),
        truncated: false,
        custom_summary: None,
    };
    
    let formatted = result.format_errors();
    assert!(formatted.contains("Errors (3)"));
    assert!(formatted.contains("[file: image.jpg]"));
    assert!(formatted.contains("[file: data.json]"));
    assert!(formatted.contains("[file: script.sh]"));
    assert!(formatted.contains("permission denied"));
}

#[test]
fn test_format_errors_positional_index() {
    use tui_framework::app::background_tasks::{BatchResult, TaskIdentifier};
    use anyhow::anyhow;
    
    let result = BatchResult {
        total: 2,
        successful: 0,
        failed: 2,
        cancelled: 0,
        success_rate: 0.0,
        errors: vec![
            (TaskIdentifier::Index(0), anyhow!("Network timeout")),
            (TaskIdentifier::Index(5), anyhow!("Connection refused")),
        ],
        results: Vec::new(),
        filtered_errors: Vec::new(),
        truncated: false,
        custom_summary: None,
    };
    
    let formatted = result.format_errors();
    assert!(formatted.contains("Errors (2)"));
    assert!(formatted.contains("[Task 0]"));
    assert!(formatted.contains("[Task 5]"));
    assert!(formatted.contains("Network timeout"));
}

#[test]
fn test_format_errors_empty() {
    use tui_framework::app::background_tasks::{BatchResult, TaskIdentifier};
    
    let result = BatchResult {
        total: 5,
        successful: 5,
        failed: 0,
        cancelled: 0,
        success_rate: 100.0,
        errors: Vec::new(),
        results: Vec::new(),
        filtered_errors: Vec::new(),
        truncated: false,
        custom_summary: None,
    };
    
    let formatted = result.format_errors();
    assert_eq!(formatted, "");
}

#[test]
fn test_with_summary() {
    use tui_framework::app::background_tasks::{BatchResult, TaskIdentifier};
    
    let result = BatchResult {
        total: 10,
        successful: 8,
        failed: 2,
        cancelled: 0,
        success_rate: 80.0,
        errors: Vec::new(),
        results: Vec::new(),
        filtered_errors: Vec::new(),
        truncated: false,
        custom_summary: None,
    };
    
    let custom = result.with_summary("Custom summary message");
    assert_eq!(custom.custom_summary, Some("Custom summary message".to_string()));
    assert_eq!(custom.generate_summary(), "Custom summary message");
}

#[test]
fn test_aggregate_results() {
    use tui_framework::app::background_tasks::{aggregate_results, BatchTaskResult, TaskIdentifier, TaskStatus};
    use anyhow::anyhow;
    
    let results = vec![
        BatchTaskResult {
            identifier: TaskIdentifier::Provided("task1".to_string()),
            status: TaskStatus::Success,
            value: Some(Box::new(1)),
            error: None,
        },
        BatchTaskResult {
            identifier: TaskIdentifier::Provided("task2".to_string()),
            status: TaskStatus::Failure,
            value: None,
            error: Some(anyhow!("Error 2")),
        },
        BatchTaskResult {
            identifier: TaskIdentifier::Provided("task3".to_string()),
            status: TaskStatus::Success,
            value: Some(Box::new(3)),
            error: None,
        },
        BatchTaskResult {
            identifier: TaskIdentifier::Provided("task4".to_string()),
            status: TaskStatus::Cancelled,
            value: None,
            error: None,
        },
    ];
    
    let aggregated = aggregate_results(results);
    
    assert_eq!(aggregated.total, 4);
    assert_eq!(aggregated.successful, 2);
    assert_eq!(aggregated.failed, 1);
    assert_eq!(aggregated.cancelled, 1);
    assert_eq!(aggregated.errors.len(), 1);
    assert!((aggregated.success_rate - 66.67).abs() < 0.1);
}

#[test]
fn test_merge_results() {
    use tui_framework::app::background_tasks::{merge_results, BatchResult, TaskIdentifier};
    use anyhow::anyhow;
    
    let result1 = BatchResult {
        total: 10,
        successful: 8,
        failed: 2,
        cancelled: 0,
        success_rate: 80.0,
        errors: vec![
            (TaskIdentifier::Index(0), anyhow!("Error 1")),
            (TaskIdentifier::Index(1), anyhow!("Error 2")),
        ],
        results: Vec::new(),
        filtered_errors: Vec::new(),
        truncated: false,
        custom_summary: None,
    };
    
    let result2 = BatchResult {
        total: 5,
        successful: 4,
        failed: 1,
        cancelled: 0,
        success_rate: 80.0,
        errors: vec![
            (TaskIdentifier::Index(0), anyhow!("Error 3")),
        ],
        results: Vec::new(),
        filtered_errors: Vec::new(),
        truncated: false,
        custom_summary: None,
    };
    
    let merged = merge_results(&[result1, result2]);
    
    assert_eq!(merged.total, 15);
    assert_eq!(merged.successful, 12);
    assert_eq!(merged.failed, 3);
    assert_eq!(merged.errors.len(), 3);
    assert!((merged.success_rate - 80.0).abs() < 0.1);
}

#[test]
fn test_merge_results_empty() {
    use tui_framework::app::background_tasks::merge_results;
    
    let merged = merge_results(&[]);
    
    assert_eq!(merged.total, 0);
    assert_eq!(merged.successful, 0);
    assert_eq!(merged.failed, 0);
    assert_eq!(merged.errors.len(), 0);
}

#[test]
fn test_aggregate_with_filter() {
    use tui_framework::app::background_tasks::{aggregate_with_filter, BatchTaskResult, TaskIdentifier, TaskStatus};
    use anyhow::anyhow;
    
    let results = vec![
        BatchTaskResult {
            identifier: TaskIdentifier::Index(0),
            status: TaskStatus::Success,
            value: None,
            error: None,
        },
        BatchTaskResult {
            identifier: TaskIdentifier::Index(1),
            status: TaskStatus::Failure,
            value: None,
            error: Some(anyhow!("Network error")),
        },
        BatchTaskResult {
            identifier: TaskIdentifier::Index(2),
            status: TaskStatus::Failure,
            value: None,
            error: Some(anyhow!("Timeout")),
        },
    ];
    
    // Filter out "Timeout" errors
    let filtered = aggregate_with_filter(results, |err| {
        !err.to_string().contains("Timeout")
    });
    
    assert_eq!(filtered.total, 3);
    assert_eq!(filtered.successful, 1);
    assert_eq!(filtered.failed, 1); // Only Network error counted
    assert_eq!(filtered.errors.len(), 1);
    assert_eq!(filtered.filtered_errors.len(), 1); // Timeout filtered
}

#[test]
fn test_aggregate_with_limit() {
    use tui_framework::app::background_tasks::{aggregate_with_limit, BatchTaskResult, TaskIdentifier, TaskStatus};
    use anyhow::anyhow;
    
    let results: Vec<_> = (0..10)
        .map(|i| BatchTaskResult {
            identifier: TaskIdentifier::Index(i),
            status: TaskStatus::Failure,
            value: None,
            error: Some(anyhow!("Error {}", i)),
        })
        .collect();
    
    let limited = aggregate_with_limit(results, Some(3));
    
    assert_eq!(limited.total, 10);
    assert_eq!(limited.failed, 10);
    assert_eq!(limited.errors.len(), 3); // Limited to 3
    assert!(limited.truncated); // Should be truncated
}

#[test]
fn test_aggregate_with_limit_no_limit() {
    use tui_framework::app::background_tasks::{aggregate_with_limit, BatchTaskResult, TaskIdentifier, TaskStatus};
    use anyhow::anyhow;
    
    let results: Vec<_> = (0..5)
        .map(|i| BatchTaskResult {
            identifier: TaskIdentifier::Index(i),
            status: TaskStatus::Failure,
            value: None,
            error: Some(anyhow!("Error {}", i)),
        })
        .collect();
    
    let unlimited = aggregate_with_limit(results, None);
    
    assert_eq!(unlimited.total, 5);
    assert_eq!(unlimited.errors.len(), 5); // All errors collected
    assert!(!unlimited.truncated);
}

#[test]
fn test_edge_case_empty_results() {
    use tui_framework::app::background_tasks::{aggregate_results, BatchTaskResult, TaskIdentifier, TaskStatus};
    
    let result = aggregate_results(Vec::new());
    
    assert_eq!(result.total, 0);
    assert_eq!(result.successful, 0);
    assert_eq!(result.failed, 0);
    assert_eq!(result.cancelled, 0);
    assert_eq!(result.success_rate, 0.0);
    assert_eq!(result.errors.len(), 0);
}

#[test]
fn test_edge_case_all_cancelled() {
    use tui_framework::app::background_tasks::{aggregate_results, BatchTaskResult, TaskIdentifier, TaskStatus};
    
    let results: Vec<_> = (0..5)
        .map(|i| BatchTaskResult {
            identifier: TaskIdentifier::Index(i),
            status: TaskStatus::Cancelled,
            value: None,
            error: None,
        })
        .collect();
    
    let result = aggregate_results(results);
    
    assert_eq!(result.total, 5);
    assert_eq!(result.successful, 0);
    assert_eq!(result.failed, 0);
    assert_eq!(result.cancelled, 5);
    // Success rate should be 0 when no successful or failed tasks
    assert_eq!(result.success_rate, 0.0);
}

#[test]
fn test_edge_case_zero_total() {
    use tui_framework::app::background_tasks::{BatchResult, TaskIdentifier};
    
    let result = BatchResult {
        total: 0,
        successful: 0,
        failed: 0,
        cancelled: 0,
        success_rate: 0.0,
        errors: Vec::new(),
        results: Vec::new(),
        filtered_errors: Vec::new(),
        truncated: false,
        custom_summary: None,
    };
    
    let summary = result.generate_summary();
    assert!(summary.contains("No tasks executed"));
    
    let errors = result.format_errors();
    assert_eq!(errors, "");
    
    assert!(result.all_succeeded()); // Empty batch is considered successful
}

#[test]
fn test_performance_summary_generation() {
    use tui_framework::app::background_tasks::{BatchResult, TaskIdentifier};
    use std::time::Instant;
    
    // Create a large batch result (10,000 tasks)
    let result = BatchResult {
        total: 10_000,
        successful: 9_500,
        failed: 500,
        cancelled: 0,
        success_rate: 95.0,
        errors: Vec::new(), // Empty errors for performance test
        results: Vec::new(),
        filtered_errors: Vec::new(),
        truncated: false,
        custom_summary: None,
    };
    
    let start = Instant::now();
    let _summary = result.generate_summary();
    let duration = start.elapsed();
    
    // Should complete in under 10ms (SC-005)
    assert!(duration.as_millis() < 10, "Summary generation took {}ms, expected < 10ms", duration.as_millis());
}

#[test]
fn test_performance_error_formatting() {
    use tui_framework::app::background_tasks::{BatchResult, TaskIdentifier};
    use anyhow::anyhow;
    use std::time::Instant;
    
    // Create result with many errors (1,000 errors)
    let errors: Vec<_> = (0..1_000)
        .map(|i| (TaskIdentifier::Index(i), anyhow!("Error {}", i)))
        .collect();
    
    let result = BatchResult {
        total: 1_000,
        successful: 0,
        failed: 1_000,
        cancelled: 0,
        success_rate: 0.0,
        errors,
        results: Vec::new(),
        filtered_errors: Vec::new(),
        truncated: false,
        custom_summary: None,
    };
    
    let start = Instant::now();
    let _formatted = result.format_errors();
    let duration = start.elapsed();
    
    // Should complete reasonably quickly (linear time complexity)
    assert!(duration.as_millis() < 100, "Error formatting took {}ms", duration.as_millis());
}

// ============================================================================
// Task Result Aggregation Tests
// ============================================================================

#[tokio::test]
async fn test_backward_compatibility_spawn_streaming() {
    use tokio::sync::mpsc;
    use tokio::time::sleep;
    use std::time::Duration;

    let mut manager = BackgroundTaskManager::new();

    // Test that existing spawn_streaming() method still works
    let token = manager.spawn_streaming(|sender, _cancel| {
        Box::pin(async move {
            sender.send("test".to_string()).await.unwrap();
            Ok(())
        })
    });
    
    // Wait a bit for message to be sent
    sleep(Duration::from_millis(10)).await;
    
    // Try to receive stream line
    let line = manager.try_recv_stream_line();
    assert_eq!(line, Some("test".to_string()));
    
    // Clean up
    manager.cancel_task(&token);
}

#[test]
fn test_format_errors_provided_identifiers() {
    use tui_framework::app::background_tasks::{BatchResult, TaskIdentifier};
    use anyhow::anyhow;
    
    let result = BatchResult {
        total: 3,
        successful: 0,
        failed: 3,
        cancelled: 0,
        success_rate: 0.0,
        errors: vec![
            (TaskIdentifier::Provided("file: image.jpg".to_string()), anyhow!("permission denied")),
            (TaskIdentifier::Provided("file: data.json".to_string()), anyhow!("invalid JSON")),
            (TaskIdentifier::Provided("file: script.sh".to_string()), anyhow!("execution failed")),
        ],
        results: Vec::new(),
        filtered_errors: Vec::new(),
        truncated: false,
        custom_summary: None,
    };
    
    let formatted = result.format_errors();
    assert!(formatted.contains("Errors (3)"));
    assert!(formatted.contains("[file: image.jpg]"));
    assert!(formatted.contains("[file: data.json]"));
    assert!(formatted.contains("[file: script.sh]"));
    assert!(formatted.contains("permission denied"));
}

#[test]
fn test_format_errors_positional_index() {
    use tui_framework::app::background_tasks::{BatchResult, TaskIdentifier};
    use anyhow::anyhow;
    
    let result = BatchResult {
        total: 2,
        successful: 0,
        failed: 2,
        cancelled: 0,
        success_rate: 0.0,
        errors: vec![
            (TaskIdentifier::Index(0), anyhow!("Network timeout")),
            (TaskIdentifier::Index(5), anyhow!("Connection refused")),
        ],
        results: Vec::new(),
        filtered_errors: Vec::new(),
        truncated: false,
        custom_summary: None,
    };
    
    let formatted = result.format_errors();
    assert!(formatted.contains("Errors (2)"));
    assert!(formatted.contains("[Task 0]"));
    assert!(formatted.contains("[Task 5]"));
    assert!(formatted.contains("Network timeout"));
}

#[test]
fn test_format_errors_empty() {
    use tui_framework::app::background_tasks::{BatchResult, TaskIdentifier};
    
    let result = BatchResult {
        total: 5,
        successful: 5,
        failed: 0,
        cancelled: 0,
        success_rate: 100.0,
        errors: Vec::new(),
        results: Vec::new(),
        filtered_errors: Vec::new(),
        truncated: false,
        custom_summary: None,
    };
    
    let formatted = result.format_errors();
    assert_eq!(formatted, "");
}

#[test]
fn test_with_summary() {
    use tui_framework::app::background_tasks::{BatchResult, TaskIdentifier};
    
    let result = BatchResult {
        total: 10,
        successful: 8,
        failed: 2,
        cancelled: 0,
        success_rate: 80.0,
        errors: Vec::new(),
        results: Vec::new(),
        filtered_errors: Vec::new(),
        truncated: false,
        custom_summary: None,
    };
    
    let custom = result.with_summary("Custom summary message");
    assert_eq!(custom.custom_summary, Some("Custom summary message".to_string()));
    assert_eq!(custom.generate_summary(), "Custom summary message");
}

#[test]
fn test_aggregate_results() {
    use tui_framework::app::background_tasks::{aggregate_results, BatchTaskResult, TaskIdentifier, TaskStatus};
    use anyhow::anyhow;
    
    let results = vec![
        BatchTaskResult {
            identifier: TaskIdentifier::Provided("task1".to_string()),
            status: TaskStatus::Success,
            value: Some(Box::new(1)),
            error: None,
        },
        BatchTaskResult {
            identifier: TaskIdentifier::Provided("task2".to_string()),
            status: TaskStatus::Failure,
            value: None,
            error: Some(anyhow!("Error 2")),
        },
        BatchTaskResult {
            identifier: TaskIdentifier::Provided("task3".to_string()),
            status: TaskStatus::Success,
            value: Some(Box::new(3)),
            error: None,
        },
        BatchTaskResult {
            identifier: TaskIdentifier::Provided("task4".to_string()),
            status: TaskStatus::Cancelled,
            value: None,
            error: None,
        },
    ];
    
    let aggregated = aggregate_results(results);
    
    assert_eq!(aggregated.total, 4);
    assert_eq!(aggregated.successful, 2);
    assert_eq!(aggregated.failed, 1);
    assert_eq!(aggregated.cancelled, 1);
    assert_eq!(aggregated.errors.len(), 1);
    assert!((aggregated.success_rate - 66.67).abs() < 0.1);
}

#[test]
fn test_merge_results() {
    use tui_framework::app::background_tasks::{merge_results, BatchResult, TaskIdentifier};
    use anyhow::anyhow;
    
    let result1 = BatchResult {
        total: 10,
        successful: 8,
        failed: 2,
        cancelled: 0,
        success_rate: 80.0,
        errors: vec![
            (TaskIdentifier::Index(0), anyhow!("Error 1")),
            (TaskIdentifier::Index(1), anyhow!("Error 2")),
        ],
        results: Vec::new(),
        filtered_errors: Vec::new(),
        truncated: false,
        custom_summary: None,
    };
    
    let result2 = BatchResult {
        total: 5,
        successful: 4,
        failed: 1,
        cancelled: 0,
        success_rate: 80.0,
        errors: vec![
            (TaskIdentifier::Index(0), anyhow!("Error 3")),
        ],
        results: Vec::new(),
        filtered_errors: Vec::new(),
        truncated: false,
        custom_summary: None,
    };
    
    let merged = merge_results(&[result1, result2]);
    
    assert_eq!(merged.total, 15);
    assert_eq!(merged.successful, 12);
    assert_eq!(merged.failed, 3);
    assert_eq!(merged.errors.len(), 3);
    assert!((merged.success_rate - 80.0).abs() < 0.1);
}

#[test]
fn test_merge_results_empty() {
    use tui_framework::app::background_tasks::merge_results;
    
    let merged = merge_results(&[]);
    
    assert_eq!(merged.total, 0);
    assert_eq!(merged.successful, 0);
    assert_eq!(merged.failed, 0);
    assert_eq!(merged.errors.len(), 0);
}

#[test]
fn test_aggregate_with_filter() {
    use tui_framework::app::background_tasks::{aggregate_with_filter, BatchTaskResult, TaskIdentifier, TaskStatus};
    use anyhow::anyhow;
    
    let results = vec![
        BatchTaskResult {
            identifier: TaskIdentifier::Index(0),
            status: TaskStatus::Success,
            value: None,
            error: None,
        },
        BatchTaskResult {
            identifier: TaskIdentifier::Index(1),
            status: TaskStatus::Failure,
            value: None,
            error: Some(anyhow!("Network error")),
        },
        BatchTaskResult {
            identifier: TaskIdentifier::Index(2),
            status: TaskStatus::Failure,
            value: None,
            error: Some(anyhow!("Timeout")),
        },
    ];
    
    // Filter out "Timeout" errors
    let filtered = aggregate_with_filter(results, |err| {
        !err.to_string().contains("Timeout")
    });
    
    assert_eq!(filtered.total, 3);
    assert_eq!(filtered.successful, 1);
    assert_eq!(filtered.failed, 1); // Only Network error counted
    assert_eq!(filtered.errors.len(), 1);
    assert_eq!(filtered.filtered_errors.len(), 1); // Timeout filtered
}

#[test]
fn test_aggregate_with_limit() {
    use tui_framework::app::background_tasks::{aggregate_with_limit, BatchTaskResult, TaskIdentifier, TaskStatus};
    use anyhow::anyhow;
    
    let results: Vec<_> = (0..10)
        .map(|i| BatchTaskResult {
            identifier: TaskIdentifier::Index(i),
            status: TaskStatus::Failure,
            value: None,
            error: Some(anyhow!("Error {}", i)),
        })
        .collect();
    
    let limited = aggregate_with_limit(results, Some(3));
    
    assert_eq!(limited.total, 10);
    assert_eq!(limited.failed, 10);
    assert_eq!(limited.errors.len(), 3); // Limited to 3
    assert!(limited.truncated); // Should be truncated
}

#[test]
fn test_aggregate_with_limit_no_limit() {
    use tui_framework::app::background_tasks::{aggregate_with_limit, BatchTaskResult, TaskIdentifier, TaskStatus};
    use anyhow::anyhow;
    
    let results: Vec<_> = (0..5)
        .map(|i| BatchTaskResult {
            identifier: TaskIdentifier::Index(i),
            status: TaskStatus::Failure,
            value: None,
            error: Some(anyhow!("Error {}", i)),
        })
        .collect();
    
    let unlimited = aggregate_with_limit(results, None);
    
    assert_eq!(unlimited.total, 5);
    assert_eq!(unlimited.errors.len(), 5); // All errors collected
    assert!(!unlimited.truncated);
}

#[test]
fn test_edge_case_empty_results() {
    use tui_framework::app::background_tasks::{aggregate_results, BatchTaskResult, TaskIdentifier, TaskStatus};
    
    let result = aggregate_results(Vec::new());
    
    assert_eq!(result.total, 0);
    assert_eq!(result.successful, 0);
    assert_eq!(result.failed, 0);
    assert_eq!(result.cancelled, 0);
    assert_eq!(result.success_rate, 0.0);
    assert_eq!(result.errors.len(), 0);
}

#[test]
fn test_edge_case_all_cancelled() {
    use tui_framework::app::background_tasks::{aggregate_results, BatchTaskResult, TaskIdentifier, TaskStatus};
    
    let results: Vec<_> = (0..5)
        .map(|i| BatchTaskResult {
            identifier: TaskIdentifier::Index(i),
            status: TaskStatus::Cancelled,
            value: None,
            error: None,
        })
        .collect();
    
    let result = aggregate_results(results);
    
    assert_eq!(result.total, 5);
    assert_eq!(result.successful, 0);
    assert_eq!(result.failed, 0);
    assert_eq!(result.cancelled, 5);
    // Success rate should be 0 when no successful or failed tasks
    assert_eq!(result.success_rate, 0.0);
}

#[test]
fn test_edge_case_zero_total() {
    use tui_framework::app::background_tasks::{BatchResult, TaskIdentifier};
    
    let result = BatchResult {
        total: 0,
        successful: 0,
        failed: 0,
        cancelled: 0,
        success_rate: 0.0,
        errors: Vec::new(),
        results: Vec::new(),
        filtered_errors: Vec::new(),
        truncated: false,
        custom_summary: None,
    };
    
    let summary = result.generate_summary();
    assert!(summary.contains("No tasks executed"));
    
    let errors = result.format_errors();
    assert_eq!(errors, "");
    
    assert!(result.all_succeeded()); // Empty batch is considered successful
}

#[test]
fn test_performance_summary_generation() {
    use tui_framework::app::background_tasks::{BatchResult, TaskIdentifier};
    use std::time::Instant;
    
    // Create a large batch result (10,000 tasks)
    let result = BatchResult {
        total: 10_000,
        successful: 9_500,
        failed: 500,
        cancelled: 0,
        success_rate: 95.0,
        errors: Vec::new(), // Empty errors for performance test
        results: Vec::new(),
        filtered_errors: Vec::new(),
        truncated: false,
        custom_summary: None,
    };
    
    let start = Instant::now();
    let _summary = result.generate_summary();
    let duration = start.elapsed();
    
    // Should complete in under 10ms (SC-005)
    assert!(duration.as_millis() < 10, "Summary generation took {}ms, expected < 10ms", duration.as_millis());
}

#[test]
fn test_performance_error_formatting() {
    use tui_framework::app::background_tasks::{BatchResult, TaskIdentifier};
    use anyhow::anyhow;
    use std::time::Instant;
    
    // Create result with many errors (1,000 errors)
    let errors: Vec<_> = (0..1_000)
        .map(|i| (TaskIdentifier::Index(i), anyhow!("Error {}", i)))
        .collect();
    
    let result = BatchResult {
        total: 1_000,
        successful: 0,
        failed: 1_000,
        cancelled: 0,
        success_rate: 0.0,
        errors,
        results: Vec::new(),
        filtered_errors: Vec::new(),
        truncated: false,
        custom_summary: None,
    };
    
    let start = Instant::now();
    let _formatted = result.format_errors();
    let duration = start.elapsed();
    
    // Should complete reasonably quickly (linear time complexity)
    assert!(duration.as_millis() < 100, "Error formatting took {}ms", duration.as_millis());
}

// ============================================================================
// Task Result Aggregation Tests
// ============================================================================

#[test]
fn test_generate_summary_all_success() {
    use tui_framework::app::background_tasks::{BatchResult, TaskIdentifier};
    
    let result = BatchResult {
        total: 45,
        successful: 45,
        failed: 0,
        cancelled: 0,
        success_rate: 100.0,
        errors: Vec::new(),
        results: Vec::new(),
        filtered_errors: Vec::new(),
        truncated: false,
        custom_summary: None,
    };
    
    let summary = result.generate_summary();
    assert!(summary.contains("All 45 tasks completed successfully") || summary.contains("45"));
    assert!(summary.contains("successfully"));
}

#[test]
fn test_generate_summary_mixed_results() {
    use tui_framework::app::background_tasks::{BatchResult, TaskIdentifier};
    use anyhow::anyhow;
    
    let result = BatchResult {
        total: 45,
        successful: 42,
        failed: 3,
        cancelled: 0,
        success_rate: 93.33,
        errors: vec![
            (TaskIdentifier::Provided("file1".to_string()), anyhow!("Error 1")),
            (TaskIdentifier::Provided("file2".to_string()), anyhow!("Error 2")),
            (TaskIdentifier::Provided("file3".to_string()), anyhow!("Error 3")),
        ],
        results: Vec::new(),
        filtered_errors: Vec::new(),
        truncated: false,
        custom_summary: None,
    };
    
    let summary = result.generate_summary();
    assert!(summary.contains("45"));
    assert!(summary.contains("42"));
    assert!(summary.contains("3"));
    assert!(summary.contains("93") || summary.contains("success rate"));
}

#[test]
fn test_generate_summary_all_failure() {
    use tui_framework::app::background_tasks::{BatchResult, TaskIdentifier};
    use anyhow::anyhow;
    
    let result = BatchResult {
        total: 5,
        successful: 0,
        failed: 5,
        cancelled: 0,
        success_rate: 0.0,
        errors: vec![
            (TaskIdentifier::Index(0), anyhow!("Error 1")),
            (TaskIdentifier::Index(1), anyhow!("Error 2")),
            (TaskIdentifier::Index(2), anyhow!("Error 3")),
            (TaskIdentifier::Index(3), anyhow!("Error 4")),
            (TaskIdentifier::Index(4), anyhow!("Error 5")),
        ],
        results: Vec::new(),
        filtered_errors: Vec::new(),
        truncated: false,
        custom_summary: None,
    };
    
    let summary = result.generate_summary();
    assert!(summary.contains("5"));
    assert!(summary.contains("0") || summary.contains("failed"));
    assert!(summary.contains("0%") || summary.contains("success rate"));
}

#[test]
fn test_generate_summary_empty_results() {
    use tui_framework::app::background_tasks::{BatchResult, TaskIdentifier};
    
    let result = BatchResult {
        total: 0,
        successful: 0,
        failed: 0,
        cancelled: 0,
        success_rate: 0.0,
        errors: Vec::new(),
        results: Vec::new(),
        filtered_errors: Vec::new(),
        truncated: false,
        custom_summary: None,
    };
    
    let summary = result.generate_summary();
    assert!(summary.contains("No tasks executed") || summary.contains("0"));
}

#[test]
fn test_format_errors_provided_identifiers() {
    use tui_framework::app::background_tasks::{BatchResult, TaskIdentifier};
    use anyhow::anyhow;
    
    let result = BatchResult {
        total: 3,
        successful: 0,
        failed: 3,
        cancelled: 0,
        success_rate: 0.0,
        errors: vec![
            (TaskIdentifier::Provided("file: image.jpg".to_string()), anyhow!("permission denied")),
            (TaskIdentifier::Provided("file: data.json".to_string()), anyhow!("invalid JSON")),
            (TaskIdentifier::Provided("file: script.sh".to_string()), anyhow!("execution failed")),
        ],
        results: Vec::new(),
        filtered_errors: Vec::new(),
        truncated: false,
        custom_summary: None,
    };
    
    let formatted = result.format_errors();
    assert!(formatted.contains("Errors (3)"));
    assert!(formatted.contains("[file: image.jpg]"));
    assert!(formatted.contains("[file: data.json]"));
    assert!(formatted.contains("[file: script.sh]"));
    assert!(formatted.contains("permission denied"));
}

#[test]
fn test_format_errors_positional_index() {
    use tui_framework::app::background_tasks::{BatchResult, TaskIdentifier};
    use anyhow::anyhow;
    
    let result = BatchResult {
        total: 2,
        successful: 0,
        failed: 2,
        cancelled: 0,
        success_rate: 0.0,
        errors: vec![
            (TaskIdentifier::Index(0), anyhow!("Network timeout")),
            (TaskIdentifier::Index(5), anyhow!("Connection refused")),
        ],
        results: Vec::new(),
        filtered_errors: Vec::new(),
        truncated: false,
        custom_summary: None,
    };
    
    let formatted = result.format_errors();
    assert!(formatted.contains("Errors (2)"));
    assert!(formatted.contains("[Task 0]"));
    assert!(formatted.contains("[Task 5]"));
    assert!(formatted.contains("Network timeout"));
}

#[test]
fn test_format_errors_empty() {
    use tui_framework::app::background_tasks::{BatchResult, TaskIdentifier};
    
    let result = BatchResult {
        total: 5,
        successful: 5,
        failed: 0,
        cancelled: 0,
        success_rate: 100.0,
        errors: Vec::new(),
        results: Vec::new(),
        filtered_errors: Vec::new(),
        truncated: false,
        custom_summary: None,
    };
    
    let formatted = result.format_errors();
    assert_eq!(formatted, "");
}

#[test]
fn test_with_summary() {
    use tui_framework::app::background_tasks::{BatchResult, TaskIdentifier};
    
    let result = BatchResult {
        total: 10,
        successful: 8,
        failed: 2,
        cancelled: 0,
        success_rate: 80.0,
        errors: Vec::new(),
        results: Vec::new(),
        filtered_errors: Vec::new(),
        truncated: false,
        custom_summary: None,
    };
    
    let custom = result.with_summary("Custom summary message");
    assert_eq!(custom.custom_summary, Some("Custom summary message".to_string()));
    assert_eq!(custom.generate_summary(), "Custom summary message");
}

#[test]
fn test_aggregate_results() {
    use tui_framework::app::background_tasks::{aggregate_results, BatchTaskResult, TaskIdentifier, TaskStatus};
    use anyhow::anyhow;
    
    let results = vec![
        BatchTaskResult {
            identifier: TaskIdentifier::Provided("task1".to_string()),
            status: TaskStatus::Success,
            value: Some(Box::new(1)),
            error: None,
        },
        BatchTaskResult {
            identifier: TaskIdentifier::Provided("task2".to_string()),
            status: TaskStatus::Failure,
            value: None,
            error: Some(anyhow!("Error 2")),
        },
        BatchTaskResult {
            identifier: TaskIdentifier::Provided("task3".to_string()),
            status: TaskStatus::Success,
            value: Some(Box::new(3)),
            error: None,
        },
        BatchTaskResult {
            identifier: TaskIdentifier::Provided("task4".to_string()),
            status: TaskStatus::Cancelled,
            value: None,
            error: None,
        },
    ];
    
    let aggregated = aggregate_results(results);
    
    assert_eq!(aggregated.total, 4);
    assert_eq!(aggregated.successful, 2);
    assert_eq!(aggregated.failed, 1);
    assert_eq!(aggregated.cancelled, 1);
    assert_eq!(aggregated.errors.len(), 1);
    assert!((aggregated.success_rate - 66.67).abs() < 0.1);
}

#[test]
fn test_merge_results() {
    use tui_framework::app::background_tasks::{merge_results, BatchResult, TaskIdentifier};
    use anyhow::anyhow;
    
    let result1 = BatchResult {
        total: 10,
        successful: 8,
        failed: 2,
        cancelled: 0,
        success_rate: 80.0,
        errors: vec![
            (TaskIdentifier::Index(0), anyhow!("Error 1")),
            (TaskIdentifier::Index(1), anyhow!("Error 2")),
        ],
        results: Vec::new(),
        filtered_errors: Vec::new(),
        truncated: false,
        custom_summary: None,
    };
    
    let result2 = BatchResult {
        total: 5,
        successful: 4,
        failed: 1,
        cancelled: 0,
        success_rate: 80.0,
        errors: vec![
            (TaskIdentifier::Index(0), anyhow!("Error 3")),
        ],
        results: Vec::new(),
        filtered_errors: Vec::new(),
        truncated: false,
        custom_summary: None,
    };
    
    let merged = merge_results(&[result1, result2]);
    
    assert_eq!(merged.total, 15);
    assert_eq!(merged.successful, 12);
    assert_eq!(merged.failed, 3);
    assert_eq!(merged.errors.len(), 3);
    assert!((merged.success_rate - 80.0).abs() < 0.1);
}

#[test]
fn test_merge_results_empty() {
    use tui_framework::app::background_tasks::merge_results;
    
    let merged = merge_results(&[]);
    
    assert_eq!(merged.total, 0);
    assert_eq!(merged.successful, 0);
    assert_eq!(merged.failed, 0);
    assert_eq!(merged.errors.len(), 0);
}

#[test]
fn test_aggregate_with_filter() {
    use tui_framework::app::background_tasks::{aggregate_with_filter, BatchTaskResult, TaskIdentifier, TaskStatus};
    use anyhow::anyhow;
    
    let results = vec![
        BatchTaskResult {
            identifier: TaskIdentifier::Index(0),
            status: TaskStatus::Success,
            value: None,
            error: None,
        },
        BatchTaskResult {
            identifier: TaskIdentifier::Index(1),
            status: TaskStatus::Failure,
            value: None,
            error: Some(anyhow!("Network error")),
        },
        BatchTaskResult {
            identifier: TaskIdentifier::Index(2),
            status: TaskStatus::Failure,
            value: None,
            error: Some(anyhow!("Timeout")),
        },
    ];
    
    // Filter out "Timeout" errors
    let filtered = aggregate_with_filter(results, |err| {
        !err.to_string().contains("Timeout")
    });
    
    assert_eq!(filtered.total, 3);
    assert_eq!(filtered.successful, 1);
    assert_eq!(filtered.failed, 1); // Only Network error counted
    assert_eq!(filtered.errors.len(), 1);
    assert_eq!(filtered.filtered_errors.len(), 1); // Timeout filtered
}

#[test]
fn test_aggregate_with_limit() {
    use tui_framework::app::background_tasks::{aggregate_with_limit, BatchTaskResult, TaskIdentifier, TaskStatus};
    use anyhow::anyhow;
    
    let results: Vec<_> = (0..10)
        .map(|i| BatchTaskResult {
            identifier: TaskIdentifier::Index(i),
            status: TaskStatus::Failure,
            value: None,
            error: Some(anyhow!("Error {}", i)),
        })
        .collect();
    
    let limited = aggregate_with_limit(results, Some(3));
    
    assert_eq!(limited.total, 10);
    assert_eq!(limited.failed, 10);
    assert_eq!(limited.errors.len(), 3); // Limited to 3
    assert!(limited.truncated); // Should be truncated
}

#[test]
fn test_aggregate_with_limit_no_limit() {
    use tui_framework::app::background_tasks::{aggregate_with_limit, BatchTaskResult, TaskIdentifier, TaskStatus};
    use anyhow::anyhow;
    
    let results: Vec<_> = (0..5)
        .map(|i| BatchTaskResult {
            identifier: TaskIdentifier::Index(i),
            status: TaskStatus::Failure,
            value: None,
            error: Some(anyhow!("Error {}", i)),
        })
        .collect();
    
    let unlimited = aggregate_with_limit(results, None);
    
    assert_eq!(unlimited.total, 5);
    assert_eq!(unlimited.errors.len(), 5); // All errors collected
    assert!(!unlimited.truncated);
}

#[test]
fn test_edge_case_empty_results() {
    use tui_framework::app::background_tasks::{aggregate_results, BatchTaskResult, TaskIdentifier, TaskStatus};
    
    let result = aggregate_results(Vec::new());
    
    assert_eq!(result.total, 0);
    assert_eq!(result.successful, 0);
    assert_eq!(result.failed, 0);
    assert_eq!(result.cancelled, 0);
    assert_eq!(result.success_rate, 0.0);
    assert_eq!(result.errors.len(), 0);
}

#[test]
fn test_edge_case_all_cancelled() {
    use tui_framework::app::background_tasks::{aggregate_results, BatchTaskResult, TaskIdentifier, TaskStatus};
    
    let results: Vec<_> = (0..5)
        .map(|i| BatchTaskResult {
            identifier: TaskIdentifier::Index(i),
            status: TaskStatus::Cancelled,
            value: None,
            error: None,
        })
        .collect();
    
    let result = aggregate_results(results);
    
    assert_eq!(result.total, 5);
    assert_eq!(result.successful, 0);
    assert_eq!(result.failed, 0);
    assert_eq!(result.cancelled, 5);
    // Success rate should be 0 when no successful or failed tasks
    assert_eq!(result.success_rate, 0.0);
}

#[test]
fn test_edge_case_zero_total() {
    use tui_framework::app::background_tasks::{BatchResult, TaskIdentifier};
    
    let result = BatchResult {
        total: 0,
        successful: 0,
        failed: 0,
        cancelled: 0,
        success_rate: 0.0,
        errors: Vec::new(),
        results: Vec::new(),
        filtered_errors: Vec::new(),
        truncated: false,
        custom_summary: None,
    };
    
    let summary = result.generate_summary();
    assert!(summary.contains("No tasks executed"));
    
    let errors = result.format_errors();
    assert_eq!(errors, "");
    
    assert!(result.all_succeeded()); // Empty batch is considered successful
}

#[test]
fn test_performance_summary_generation() {
    use tui_framework::app::background_tasks::{BatchResult, TaskIdentifier};
    use std::time::Instant;
    
    // Create a large batch result (10,000 tasks)
    let result = BatchResult {
        total: 10_000,
        successful: 9_500,
        failed: 500,
        cancelled: 0,
        success_rate: 95.0,
        errors: Vec::new(), // Empty errors for performance test
        results: Vec::new(),
        filtered_errors: Vec::new(),
        truncated: false,
        custom_summary: None,
    };
    
    let start = Instant::now();
    let _summary = result.generate_summary();
    let duration = start.elapsed();
    
    // Should complete in under 10ms (SC-005)
    assert!(duration.as_millis() < 10, "Summary generation took {}ms, expected < 10ms", duration.as_millis());
}

#[test]
fn test_performance_error_formatting() {
    use tui_framework::app::background_tasks::{BatchResult, TaskIdentifier};
    use anyhow::anyhow;
    use std::time::Instant;
    
    // Create result with many errors (1,000 errors)
    let errors: Vec<_> = (0..1_000)
        .map(|i| (TaskIdentifier::Index(i), anyhow!("Error {}", i)))
        .collect();
    
    let result = BatchResult {
        total: 1_000,
        successful: 0,
        failed: 1_000,
        cancelled: 0,
        success_rate: 0.0,
        errors,
        results: Vec::new(),
        filtered_errors: Vec::new(),
        truncated: false,
        custom_summary: None,
    };
    
    let start = Instant::now();
    let _formatted = result.format_errors();
    let duration = start.elapsed();
    
    // Should complete reasonably quickly (linear time complexity)
    assert!(duration.as_millis() < 100, "Error formatting took {}ms", duration.as_millis());
}

// ============================================================================
// Task Result Aggregation Tests
// ============================================================================

