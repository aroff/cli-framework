//! Integration tests for task result aggregation utilities

use cli_framework::app::background_tasks::{
    task_definition, BackgroundTaskManager, BatchResult, aggregate_results, merge_results,
};

#[tokio::test]
async fn test_complete_batch_processing_summary_workflow() {
    let mut manager = BackgroundTaskManager::new();

    // Create a batch of 45 tasks with 3 failures
    let tasks: Vec<_> = (0..45)
        .map(|i| {
            task_definition(
                move || {
                    let idx = i;
                    async move {
                        if idx < 3 {
                            Err(anyhow::anyhow!("Task {} failed", idx))
                        } else {
                            Ok(format!("result-{}", idx))
                        }
                    }
                },
                Some(format!("file-{}", i)),
            )
        })
        .collect();

    let result = manager.spawn_batch(tasks, None).await;

    // Verify aggregated statistics
    assert_eq!(result.total, 45);
    assert_eq!(result.successful, 42);
    assert_eq!(result.failed, 3);
    assert_eq!(result.errors.len(), 3);

    // Test summary generation
    let summary = result.generate_summary();
    assert!(summary.contains("45"));
    assert!(summary.contains("42"));
    assert!(summary.contains("3"));

    // Test error formatting
    let error_output = result.format_errors();
    assert!(error_output.contains("Errors (3)"));
    assert!(error_output.contains("file-0") || error_output.contains("file-1") || error_output.contains("file-2"));

    // Test all_succeeded check
    assert!(!result.all_succeeded());
    assert!(result.has_failures());
}

#[tokio::test]
async fn test_migration_operations_reporting_workflow() {
    let mut manager = BackgroundTaskManager::new();

    // Simulate migration operations
    let tasks: Vec<_> = (0..100)
        .map(|i| {
            task_definition(
                move || {
                    let idx = i;
                    async move {
                        if idx < 5 {
                            Err(anyhow::anyhow!("Migration {} failed", idx))
                        } else {
                            Ok(format!("migrated-{}", idx))
                        }
                    }
                },
                Some(format!("migration-{}", i)),
            )
        })
        .collect();

    let result = manager.spawn_batch(tasks, None).await;

    assert_eq!(result.total, 100);
    assert_eq!(result.successful, 95);
    assert_eq!(result.failed, 5);

    // Verify success rate calculation
    let expected_rate = (95.0 / 100.0) * 100.0;
    assert!((result.success_rate - expected_rate).abs() < 0.01);

    // Test error formatting with all errors
    let error_output = result.format_errors();
    assert!(error_output.contains("Errors (5)"));
}

#[tokio::test]
async fn test_merging_multiple_migration_operations() {
    let mut manager = BackgroundTaskManager::new();

    // First migration batch
    let tasks1: Vec<_> = (0..50)
        .map(|i| {
            task_definition(
                move || async move { Ok(format!("migration1-{}", i)) },
                Some(format!("migration1-{}", i)),
            )
        })
        .collect();

    let result1 = manager.spawn_batch(tasks1, None).await;

    // Second migration batch
    let tasks2: Vec<_> = (0..30)
        .map(|i| {
            task_definition(
                move || {
                    let idx = i;
                    async move {
                        if idx < 2 {
                            Err(anyhow::anyhow!("Migration2 {} failed", idx))
                        } else {
                            Ok(format!("migration2-{}", idx))
                        }
                    }
                },
                Some(format!("migration2-{}", i)),
            )
        })
        .collect();

    let result2 = manager.spawn_batch(tasks2, None).await;

    // Merge results
    let merged = merge_results(&[result1, result2]);

    assert_eq!(merged.total, 80);
    assert_eq!(merged.successful, 78);
    assert_eq!(merged.failed, 2);
    assert_eq!(merged.errors.len(), 2);

    // Verify success rate recalculation
    let expected_rate = (78.0 / 80.0) * 100.0;
    assert!((merged.success_rate - expected_rate).abs() < 0.01);
}

